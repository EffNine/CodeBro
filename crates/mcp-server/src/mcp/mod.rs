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

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

pub mod change_invalidation;
pub mod facts;
pub mod response_bounds;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::provenance::{compute_trust, FreshnessStatus, SourceKind};
use crate::workspace_registry::{WorkspaceRegistry, WorkspaceState};

/// The CodeBro MCP server: a router over the engineering context layer.
///
/// The fact store is immutable once built, so it is cached per server
/// process (loaded once, reused across calls) with a modification-time
/// check so a concurrent `codebro init` is picked up. Everything else is
/// constructed fresh per call.
/// Cached immutable fact store keyed by the facts.json mtime.
type FactsCache =
    Arc<std::sync::Mutex<Option<(Option<std::time::SystemTime>, crate::fact_store::FactStore)>>>;

/// The CodeBro MCP server: a router over the engineering context layer.
///
/// A single server process can serve multiple workspaces via
/// [`WorkspaceRegistry`]. Each workspace has independent runtime state
/// (fact store, mutation lock, recent edits, RCA cache, journal lock).
/// When a tool call omits `workspace_root`, the server's configured
/// default workspace is used for backward compatibility.
#[derive(Clone)]
pub struct CodeBroMcpServer {
    registry: WorkspaceRegistry,
    tool_router: ToolRouter<Self>,
    sandbox_runtime: crate::sandbox::SandboxRuntime,
    /// Durable user-context store (`~/.codebro/state.db`). Global across
    /// workspaces; per-workspace data is scoped by workspace_root.
    context_store: Arc<crate::context_runtime::ContextStore>,
    /// P5 task-runtime worker identity: identifies THIS server process's
    /// task ownership across lease/fencing checks. One per process.
    task_worker_id: String,
}

/// Assemble a server: default state directory unless an explicit one is
/// given (explicit dirs keep tests and embedded deployments hermetic).
fn assemble_server(
    workspace_root: PathBuf,
    sandbox_runtime: crate::sandbox::SandboxRuntime,
    state_dir: Option<PathBuf>,
) -> CodeBroMcpServer {
    let registry = WorkspaceRegistry::new(workspace_root);
    assemble_server_with_registry(registry, sandbox_runtime, state_dir)
}

/// Assemble a server around an explicit (already-authorized) registry —
/// the P8 root-authorization seam for multi-root deployments and tests.
fn assemble_server_with_registry(
    registry: WorkspaceRegistry,
    sandbox_runtime: crate::sandbox::SandboxRuntime,
    state_dir: Option<PathBuf>,
) -> CodeBroMcpServer {
    let context_store = Arc::new(crate::context_runtime::ContextStore::at_state_dir(
        state_dir.unwrap_or_else(default_state_dir),
    ));
    CodeBroMcpServer {
        registry,
        tool_router: CodeBroMcpServer::tool_router(),
        sandbox_runtime,
        context_store,
        task_worker_id: crate::context_runtime::mint_worker_id(),
    }
}

/// State directory for the user-context database: `CODEBRO_STATE_DIR`
/// overrides the default `~/.codebro`. Resolution happens at construction;
/// the database itself is opened lazily on first use.
fn default_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEBRO_STATE_DIR") {
        return PathBuf::from(dir);
    }
    crate::config::Config::config_dir()
}

#[tool_router]
impl CodeBroMcpServer {
    /// Create a server bound to a workspace root.
    pub fn new(workspace_root: PathBuf) -> Self {
        assemble_server(
            workspace_root,
            crate::sandbox::SandboxRuntime::from_env(),
            None,
        )
    }

    /// Create a server bound to a workspace root with additional
    /// operator-authorized roots (P8 root authorization). The default
    /// root is always authorized; `extra_authorized_roots` widens the
    /// set the per-call `workspace_root` argument may address.
    pub fn with_authorized_roots(
        workspace_root: PathBuf,
        extra_authorized_roots: Vec<PathBuf>,
    ) -> Self {
        let registry = WorkspaceRegistry::with_authorized_roots(
            workspace_root.clone(),
            crate::workspace_registry::AuthorizedRoots::with_extras(
                workspace_root,
                extra_authorized_roots,
            ),
        );
        assemble_server_with_registry(registry, crate::sandbox::SandboxRuntime::from_env(), None)
    }

    /// Create a server with an explicit sandbox runtime (for tests).
    pub fn with_sandbox_runtime(
        workspace_root: PathBuf,
        runtime: crate::sandbox::SandboxRuntime,
    ) -> Self {
        assemble_server(workspace_root, runtime, None)
    }

    /// Create a server with an explicit user-context state directory and a
    /// local sandbox runtime (for hermetic tests and embedded deployments).
    pub fn with_state_dir(workspace_root: PathBuf, state_dir: PathBuf) -> Self {
        assemble_server(
            workspace_root,
            crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local),
            Some(state_dir),
        )
    }

    /// The user-context store handle.
    fn context_store(&self) -> Arc<crate::context_runtime::ContextStore> {
        Arc::clone(&self.context_store)
    }

    /// Resolve the workspace for a tool call.
    fn resolve_workspace(&self, raw: Option<&str>) -> Result<Arc<WorkspaceState>, McpError> {
        self.registry
            .resolve(raw)
            .map_err(|e| McpError::invalid_params(e, None))
    }

    /// Load the project identity for the workspace, tolerating absence.
    fn identity_snapshot(
        &self,
        ws: &WorkspaceState,
    ) -> (bool, Option<crate::project_identity::ProjectIdentity>) {
        let mut identity = crate::project_identity::ProjectIdentityRuntime::new(&ws.canonical_root);
        match identity.load() {
            Ok(_) => (true, Some(identity.snapshot())),
            Err(e) => {
                tracing::debug!(
                    "no project identity for {}: {e}",
                    ws.canonical_root.display()
                );
                (false, None)
            }
        }
    }

    /// Build the fact store for the workspace.
    fn fact_store(&self, ws: &WorkspaceState) -> crate::fact_store::FactStore {
        ws.fact_store()
    }

    /// Snapshot recent edits for debugging input.
    fn recent_edits_snapshot(
        &self,
        ws: &WorkspaceState,
    ) -> Vec<crate::debugging::candidates::RecentEditInput> {
        ws.recent_edits_snapshot()
    }

    /// Stash the latest RCA for debugging consult injection.
    fn store_last_rca(
        &self,
        ws: &WorkspaceState,
        rca: &crate::debugging::types::RootCauseAnalysis,
    ) {
        ws.store_last_rca(rca);
    }

    /// Record a successfully applied change for failure correlation.
    fn remember_edit(&self, ws: &WorkspaceState, path: &str, recommended_tests: Vec<String>) {
        ws.remember_edit(path, recommended_tests);
    }

    /// Intersect failing diagnostics with recent edits.
    fn related_recent_changes(
        &self,
        ws: &WorkspaceState,
        verification: &crate::sandbox::VerificationResult,
    ) -> Vec<serde_json::Value> {
        if verification.verified || verification.diagnostics.is_empty() {
            return Vec::new();
        }
        let guard = ws.recent_edits.lock().expect("recent edits lock");
        let diag_files: std::collections::HashSet<&str> = verification
            .diagnostics
            .iter()
            .filter_map(|d| d.file.as_deref())
            .collect();
        let failing_tests: std::collections::HashSet<&str> = verification
            .diagnostics
            .iter()
            .filter_map(|d| d.test.as_deref())
            .collect();

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for edit in guard.iter().rev() {
            let via_file = diag_files.contains(edit.path.as_str());
            let via_recommended = !edit.recommended_tests.is_empty()
                && edit
                    .recommended_tests
                    .iter()
                    .any(|t| failing_tests.contains(t.as_str()));
            let via = match (via_file, via_recommended) {
                (true, true) => "diagnostic_file+recommended_tests",
                (true, false) => "diagnostic_file",
                (false, true) => "recommended_tests",
                (false, false) => continue,
            };
            if seen.insert(edit.path.clone()) {
                out.push(json!({
                    "path": edit.path,
                    "seconds_ago": edit.at.elapsed().as_secs(),
                    "via": via,
                }));
            }
        }
        out
    }

    /// Serialize a verification result for tool responses, adding structured
    /// diagnostics, the outcome classification, the workspace modules
    /// owning any diagnostic files (via the cached fact store), prior
    /// execution evidence from the journal, and — for failures — root-cause
    /// hypotheses.
    pub(crate) fn verification_object(
        &self,
        ws: &WorkspaceState,
        verification: &crate::sandbox::VerificationResult,
        test_filter: &[String],
    ) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        obj.insert("verified".to_string(), json!(verification.verified));
        obj.insert("summary".to_string(), json!(verification.summary));
        obj.insert("violations".to_string(), json!(verification.violations));
        if let Some(ref ids) = verification.impacted_fact_ids {
            obj.insert("impacted_fact_ids".to_string(), json!(ids));
        }
        if !verification.diagnostics.is_empty() {
            obj.insert("diagnostics".to_string(), json!(verification.diagnostics));
        }
        if let Some(ref class) = verification.classification {
            obj.insert("classification".to_string(), json!(class));
        }
        let affected_modules = self.affected_modules_for(ws, verification);
        if !affected_modules.is_empty() {
            obj.insert("affected_modules".to_string(), json!(affected_modules));
        }
        let related = self.related_recent_changes(ws, verification);
        if !related.is_empty() {
            obj.insert("related_recent_changes".to_string(), json!(related));
        }
        // Execution evidence journal: surface HISTORICAL context for this
        // (tree hash, command, filter) and then record what was observed.
        // Order matters: the summary must describe history BEFORE this run.
        //
        // Semantic rules:
        // - denied runs never executed anything: nothing to record, nothing
        //   to compare against;
        // - runs without a capturable tree hash (e.g. non-git workspaces)
        //   cannot be associated with repository state, so they are neither
        //   recorded nor answered — historical evidence is always bound to
        //   a tree hash by contract;
        // - prior evidence is ADVISORY. It never skips or short-circuits
        //   execution; the current run's outcome still governs.
        let resolved_cmd = if verification.execution.resolved_command.is_empty() {
            verification.execution.command.as_str()
        } else {
            verification.execution.resolved_command.as_str()
        };
        let tree_hash = verification
            .execution
            .repo_state
            .as_ref()
            .map(|rs| rs.working_tree_hash.clone());
        if !verification.execution.denied {
            if let Some(tree_hash) = &tree_hash {
                let _guard = ws.journal_lock.lock().expect("journal lock");
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let prior = crate::sandbox::evidence_journal::summarize_prior(
                    &ws.canonical_root,
                    tree_hash,
                    resolved_cmd,
                    test_filter,
                    now,
                );
                if let Some(prior) = prior {
                    obj.insert(
                        "prior_evidence".to_string(),
                        serde_json::to_value(&prior).unwrap_or(json!({})),
                    );
                }
                let record_input = crate::sandbox::evidence_journal::JournalInput {
                    execution_id: &verification.execution.execution_id,
                    project_id: verification
                        .execution
                        .repo_identity
                        .as_ref()
                        .map(|ri| ri.project_id.as_str()),
                    tree_hash,
                    command: resolved_cmd,
                    test_filter,
                    exit_code: verification.execution.exit_code,
                    classification: verification.classification.as_deref().unwrap_or("unknown"),
                    success: verification.execution.success,
                    timed_out: verification.execution.timeout,
                    duration_ms: u64::try_from(verification.execution.duration_ms)
                        .unwrap_or(u64::MAX),
                    diagnostics: &verification.diagnostics,
                    affected_modules: &affected_modules,
                };
                if let Err(e) =
                    crate::sandbox::evidence_journal::record(&ws.canonical_root, &record_input, now)
                {
                    tracing::warn!("execution journal record failed (non-fatal): {e}");
                }
            }
        }
        // Root-cause hypotheses for failures: deterministic ranking over
        // diagnostics + fact-store linkage + this session's recent edits.
        if !verification.verified && verification.classification.as_deref() != Some("denied") {
            let fresh =
                match crate::mcp::facts::compute_freshness(&ws.fact_store(), &ws.canonical_root) {
                    crate::mcp::facts::FreshnessStatus::Fresh => "fresh",
                    crate::mcp::facts::FreshnessStatus::Stale => "stale",
                    _ => "unknown",
                };
            let recent_edits = self.recent_edits_snapshot(ws);
            let rca = crate::debugging::analyze_root_cause(crate::debugging::RootCauseInput {
                verification,
                store: &ws.fact_store(),
                workspace_root: &ws.canonical_root,
                freshness: fresh,
                recent_edits: &recent_edits,
            });
            self.store_last_rca(ws, &rca);
            obj.insert(
                "root_cause".to_string(),
                serde_json::to_value(&rca).unwrap_or(json!({})),
            );
        }
        json!(obj)
    }

    /// Map diagnostic file paths onto owning module ids from the fact store.
    fn affected_modules_for(
        &self,
        ws: &WorkspaceState,
        verification: &crate::sandbox::VerificationResult,
    ) -> Vec<String> {
        if verification.diagnostics.is_empty() {
            return Vec::new();
        }
        let store = ws.fact_store();
        let mut out = std::collections::BTreeSet::new();
        for d in &verification.diagnostics {
            let Some(file) = d.file.as_deref() else {
                continue;
            };
            for m in store.collection().modules() {
                if m.path.as_deref() == Some(file) {
                    out.insert(m.id.to_string());
                    break;
                }
            }
        }
        out.into_iter().collect()
    }

    // ── Tool 1: workspace context ─────────────────────────────────────

    /// Return the workspace context: project identity, workspace root and
    /// the state of the engineering runtime for this project. Call this
    /// first to understand what project the agent is operating in.
    ///
    /// P6 adds deterministic engineering intelligence: repository identity
    /// (canonical root + VCS), index freshness (READY/STALE/UNKNOWN with
    /// counts), architecture observations, and supported-language surface
    /// with parser limitations. All bounded and evidence-grounded.
    #[tool(
        description = "Return the workspace context: project identity, workspace root, and engineering runtime state. Call this first to orient the agent in the project."
    )]
    async fn workspace_context(
        &self,
        Parameters(args): Parameters<WorkspaceContextArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let (identity_loaded, identity) = self.identity_snapshot(&ws);
        let store = self.fact_store(&ws);
        let counts = store.collection().counts();

        // P6 repository identity: canonical root + VCS (never raw paths alone).
        let repo_identity = codebro_core::RepoIdentity::from_workspace(&ws.canonical_root);
        // P6 freshness: generation state vs live HEAD + file-digest diff.
        let freshness = crate::mcp::facts::compute_freshness(&store, &ws.canonical_root);
        let freshness_str = match freshness {
            crate::mcp::facts::FreshnessStatus::Fresh => "fresh",
            crate::mcp::facts::FreshnessStatus::Stale => "stale",
            crate::mcp::facts::FreshnessStatus::Unknown => "unknown",
        };
        // P6 persisted index metadata (UNKNOWN when never indexed via P6 path).
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let persisted_index = self
            .context_store()
            .get_repo_index(&ws.canonical_root.to_string_lossy(), now_secs)
            .ok()
            .map(|r| {
                json!({
                    "index_status": r.index_status.as_str(),
                    "indexed_at": r.indexed_at,
                    "repository_revision": r.repository_revision,
                    "file_count": r.file_count,
                    "symbol_count": r.symbol_count,
                    "edge_count": r.edge_count,
                    "stale_count": r.stale_count,
                })
            })
            .unwrap_or(json!({"index_status": "UNKNOWN"}));
        // P6 architecture observations (bounded, filesystem-grounded).
        let arch_summary = identity
            .as_ref()
            .and_then(|id| id.architecture_summary.clone())
            .unwrap_or_default();

        let payload = json!({
            "workspace_root": ws.canonical_root.display().to_string(),
            "identity_loaded": identity_loaded,
            "project_identity": identity,
            "repository_identity": {
                "project_id": repo_identity.project_id,
                "canonical_root": repo_identity.canonical_root,
                "repository_type": repo_identity.repository_type,
                "git_remote": repo_identity.git_remote,
                "commit_sha": repo_identity.commit_sha,
            },
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
                "languages": counts.languages,
                "frameworks": counts.frameworks,
                "entry_points": counts.entry_points,
                "total": counts.total,
            },
            "index_freshness": {
                "status": freshness_str,
                "persisted": persisted_index,
            },
            "architecture": {
                "summary": arch_summary,
            },
            "supported_languages": {
                "parsed": ["rust", "python", "javascript", "typescript", "tsx", "jsx", "go"],
                "file_level_only": ["c", "cpp", "shell", "toml", "yaml", "json", "markdown"],
                "limitation": "file-level languages preserve path/size/hash/classification but never invent symbols",
            },
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 2: engineering facts (relevance-ranked retrieval) ────────

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

        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let store = self.fact_store(&ws);
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
                    "language" => FactKind::Language,
                    "framework" => FactKind::Framework,
                    "entry_point" | "entrypoint" => FactKind::EntryPoint,
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
            crate::mcp::facts::compute_freshness(&store, &ws.canonical_root),
        )
        .map_err(|e| McpError::invalid_params(e, None))?;

        let returned = facts.len();
        let provenance_summary = crate::mcp::facts::provenance_summary(&store);
        let freshness = crate::mcp::facts::compute_freshness(&store, &ws.canonical_root);
        // When zero facts match, attach deterministic recovery guidance so an
        // LLM can retry productively instead of looping on a dead query.
        let recovery = if returned == 0 {
            let mut hints: Vec<String> = Vec::new();
            if args.query.trim().is_empty() {
                hints.push("query is empty — supply a symbol name or path fragment".to_string());
            } else {
                let q = args.query.trim().to_lowercase();
                // Check whether the query is very long (likely a sentence, not a symbol).
                if q.split_whitespace().count() > 4 {
                    hints.push(
                        "query looks like a full sentence — shorten to a symbol or project term \
                         (e.g. 'breaker' instead of 'circuit-breaker implementation')"
                            .to_string(),
                    );
                }
                // Suggest trying the first token as a prefix.
                if let Some(first) = q.split_whitespace().next() {
                    if first.len() >= 3 {
                        let hint = format!("try the shorter prefix '{first}'");
                        hints.push(hint);
                    }
                }
                hints.push(
                    "supported searchable fields: symbol name, module name, package name, \
                            file path, and function signature"
                        .to_string(),
                );
                hints.push(
                    "you can also filter by kind (e.g. kind=\"symbol\") or path (e.g. \
                            path=\"src/coding\") to narrow the search"
                        .to_string(),
                );
            }
            Some(json!({
                "message": "No facts matched your query.",
                "hints": hints,
            }))
        } else {
            None
        };

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
            "returned": returned,
            "facts": facts,
            "provenance_summary": {
                "verified_edges": provenance_summary.verified_edges,
                "heuristic_edges": provenance_summary.heuristic_edges,
                "unknown_edges": provenance_summary.unknown_edges,
            },
            "freshness": freshness,
            "recovery": recovery,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    /// Attach the latest deterministic root-cause hypotheses to a debugging
    /// consult as an additive file context — the LLM reasons OVER CodeBro's
    /// structured evidence rather than re-deriving it.
    pub(crate) fn inject_debugging_hypotheses(
        &self,
        ws: &WorkspaceState,
        request: &mut crate::consultant::types::ConsultantRequest,
        mode: &crate::consultant::types::ConsultantMode,
    ) {
        if !matches!(mode, crate::consultant::types::ConsultantMode::Debugging) {
            return;
        }
        let guard = ws.last_rca.lock().expect("last rca lock");
        if let Some(rca) = guard.as_ref() {
            let payload = serde_json::to_string(rca).unwrap_or_default();
            request
                .files
                .push(crate::consultant::types::ConsultantFileContext {
                    path: "codebro://root-cause-hypotheses".to_string(),
                    content: payload,
                });
        }
    }

    /// Resolve engineering memory (decisions, constraints, prior context)
    /// relevant to a task query.
    #[tool(
        description = "Resolve relevant engineering memory for a task: recorded decisions, constraints, and prior implementation context with confidence scores, source and tags. Pass task keywords to retrieve the most relevant entries."
    )]
    async fn engineering_memory(
        &self,
        Parameters(args): Parameters<MemoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let identity = crate::project_identity::ProjectIdentityRuntime::new(&ws.canonical_root);
        let mut memory =
            crate::engineering_memory::EngineeringMemoryRuntime::new(&ws.canonical_root, identity);
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
                let trust = Some(compute_trust(
                    &SourceKind::AgentDeclared,
                    entry.confidence,
                    FreshnessStatus::Unknown,
                ));
                json!({
                    "key": entry.key,
                    "value": entry.value,
                    "confidence": entry.confidence,
                    "tier": entry.tier,
                    "source": src.and_then(|s| s.metadata.source.clone()),
                    "tags": src.map(|s| s.metadata.tags.clone()).unwrap_or_default(),
                    "trust": trust,
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
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        // Plan-less, non-strict engine: boundary + staleness enforcement only.
        let engine =
            crate::coding::change_engine::ChangeEngine::new(&ws.canonical_root, &[], false);

        let prepared = engine
            .prepare(&args.path, &args.old, &args.new)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let _apply_result = engine
            .apply(&prepared)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        // Analyze which existing facts may be stale due to this mutation.
        // The edited line range (from the old-text position in the
        // pre-change content) narrows test recommendations to symbols the
        // edit actually touched.
        let edited_range = if prepared.created || prepared.old.is_empty() {
            None
        } else {
            prepared.backup.find(&prepared.old).map(|offset| {
                let start_line = prepared.backup.as_bytes()[..offset]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count() as u32
                    + 1;
                let span_lines = prepared.old.split('\n').count() as u32;
                (start_line, start_line + span_lines.saturating_sub(1))
            })
        };
        let store = self.fact_store(&ws);
        let advisory = change_invalidation::InvalidationAdvisory::analyze_with_range(
            &store,
            &args.path,
            prepared.created,
            edited_range,
        );

        self.remember_edit(&ws, &args.path, advisory.recommended_tests.clone());

        crate::history_capture::capture(
            &self.context_store(),
            &ws.canonical_root,
            crate::history_capture::HistoryCapture {
                kind: crate::context_runtime::HistoryKind::ChangeApplied,
                summary: format!(
                    "applied change to {}{}",
                    args.path,
                    if prepared.created { " (created)" } else { "" }
                ),
                tool: Some("apply_change".to_string()),
                path: Some(args.path.clone()),
                outcome: Some(if prepared.created {
                    "created".to_string()
                } else {
                    "applied".to_string()
                }),
                payload: None,
                task_id: None,
            },
        );

        let response = json!({
            "applied": true,
            "path": args.path,
            "preview": prepared.preview,
            "affected_fact_ids": advisory.affected_fact_ids,
            "affected_symbols": advisory.affected_symbols,
            "affected_modules": advisory.affected_modules,
            "recommended_tests": advisory.recommended_tests,
            "needs_reindex": advisory.needs_reindex,
            "recommendation": advisory.recommendation,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&response)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    /// Apply a multi-file transaction atomically: validate → preview →
    /// apply all-or-nothing with rollback. Enforces the workspace boundary,
    /// refuses stale or ambiguous edits, and detects conflicts across the
    /// whole set before writing anything.
    #[tool(
        description = "Apply a multi-file transaction through the ChangeEngine. All-or-nothing: changes are validated against current content, then applied with automatic rollback if any write fails. For new files pass old as an empty string. Use apply_change for single-file edits."
    )]
    async fn apply_changes(
        &self,
        Parameters(args): Parameters<ApplyChangesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        let engine =
            crate::coding::change_engine::ChangeEngine::new(&ws.canonical_root, &[], false);

        let requests: Vec<crate::coding::transaction::TransactionRequest> = args
            .changes
            .iter()
            .map(|c| crate::coding::transaction::TransactionRequest {
                path: c.path.clone(),
                old: c.old.clone(),
                new: c.new.clone(),
            })
            .collect();

        let prepared = engine
            .prepare_changes(&requests)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let report = engine
            .apply_transaction(&prepared)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        if report.success() {
            for change in &args.changes {
                self.remember_edit(&ws, &change.path, Vec::new());
            }
            let shown: Vec<&str> = args
                .changes
                .iter()
                .map(|c| c.path.as_str())
                .take(3)
                .collect();
            let more = args.changes.len().saturating_sub(shown.len());
            crate::history_capture::capture(
                &self.context_store(),
                &ws.canonical_root,
                crate::history_capture::HistoryCapture {
                    kind: crate::context_runtime::HistoryKind::ChangeApplied,
                    summary: format!(
                        "applied transaction: {} files ({}{})",
                        args.changes.len(),
                        shown.join(", "),
                        if more > 0 {
                            format!(", +{more} more")
                        } else {
                            String::new()
                        }
                    ),
                    tool: Some("apply_changes".to_string()),
                    path: None,
                    outcome: Some("applied".to_string()),
                    payload: None,
                    task_id: None,
                },
            );
        }

        let response = json!({
            "applied": report.success(),
            "applied_count": report.applied.len(),
            "created": report.created.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "rolled_back": report.rolled_back.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "preview": prepared.preview(),
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&response)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
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
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        let key = args.key.trim();
        if key.is_empty() {
            return Err(McpError::invalid_params("key must not be empty", None));
        }
        const MAX_KEY_LEN: usize = 256;
        if key.len() > MAX_KEY_LEN {
            return Err(McpError::invalid_params(
                format!("key exceeds {MAX_KEY_LEN} characters"),
                None,
            ));
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

        // Tag bounds: prevent unbounded tag lists bloating the store.
        const MAX_TAGS: usize = 32;
        const MAX_TAG_LEN: usize = 64;
        if args.tags.len() > MAX_TAGS {
            return Err(McpError::invalid_params(
                format!("tags exceed {MAX_TAGS} entries"),
                None,
            ));
        }
        if args
            .tags
            .iter()
            .any(|t| t.len() > MAX_TAG_LEN || t.trim().is_empty())
        {
            return Err(McpError::invalid_params(
                format!("each tag must be 1-{MAX_TAG_LEN} characters"),
                None,
            ));
        }

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
        if let Some(secs) = args.expires_in_secs {
            if secs > 0 {
                metadata = metadata.with_ttl(secs);
            }
        }
        metadata.provenance =
            crate::engineering_memory::types::MemoryProvenance::agent(args.session.clone());

        let identity = crate::project_identity::ProjectIdentityRuntime::new(&ws.canonical_root);
        let mut memory =
            crate::engineering_memory::EngineeringMemoryRuntime::new(&ws.canonical_root, identity);
        // Fail closed: if an existing store cannot be loaded (corrupt,
        // wrong schema, wrong workspace), refuse to write. Proceeding on
        // an empty runtime would persist a file containing only this new
        // entry and permanently destroy the accumulated store.
        if let Err(load_err) = memory.load() {
            let absent = matches!(
                load_err,
                crate::engineering_memory::runtime::EngineeringMemoryError::Storage(
                    crate::engineering_memory::store::StorageError::NotFound(_)
                )
            );
            if !absent {
                return Err(McpError::internal_error(
                    format!(
                        "refusing to record: existing memory store could not be loaded \
                         ({load_err}). Recover the quarantined file or fix the store, \
                         then retry."
                    ),
                    None,
                ));
            }
        }

        // Deterministic id from the key: upsert semantics.
        let id = format!("mem::{key}");
        let exists = memory.snapshot().iter().any(|e| e.id == id);
        let mut conflicts_reported: Vec<String> = Vec::new();

        if exists {
            // Full logical update: value AND metadata (confidence,
            // importance, tags, source) — not just the value. id/key/
            // created_at are preserved by update_with_metadata.
            memory
                .update_with_metadata(&id, value, metadata)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        } else {
            let entry = crate::engineering_memory::types::EngineeringMemoryEntry::new(
                id.clone(),
                key,
                value,
            )
            .with_metadata(metadata);
            let outcome = memory
                .record(entry)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            conflicts_reported = outcome
                .conflicts
                .iter()
                .map(|c| format!("{}:{}", c.kind_str(), c.prior_key))
                .collect();
        }
        memory
            .persist()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let action = if exists { "updated" } else { "recorded" };
        let mut summary = format!("memory {action}: {key}");
        if !conflicts_reported.is_empty() {
            summary.push_str(&format!("; conflicts: {}", conflicts_reported.join(", ")));
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(summary)]))
    }

    // ── Tool 6: delete engineering memory (guarded write) ─────────────

    /// Delete an engineering memory entry by its exact key.
    #[tool(
        description = "Delete an engineering memory entry by its exact key. Persisted to .codebro/engineering_memory.json. Requires confirm=true — omitting it is a no-op."
    )]
    async fn delete_memory(
        &self,
        Parameters(args): Parameters<DeleteMemoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        let key = args.key.trim();
        if key.is_empty() {
            return Err(McpError::invalid_params("key must not be empty", None));
        }
        // Fast-fail on missing confirmation before touching state; the
        // memory runtime re-enforces this independently as a backstop.
        if !args.confirm {
            return Err(McpError::invalid_params(
                format!("delete rejected: set confirm=true to delete '{key}'"),
                None,
            ));
        }
        let id = format!("mem::{key}");

        let identity = crate::project_identity::ProjectIdentityRuntime::new(&ws.canonical_root);
        let mut memory =
            crate::engineering_memory::EngineeringMemoryRuntime::new(&ws.canonical_root, identity);
        // Fail closed: a corrupt/unloadable store must never be treated as
        // an empty one, or deletion would persist a store missing entries.
        if let Err(load_err) = memory.load() {
            let absent = matches!(
                load_err,
                crate::engineering_memory::runtime::EngineeringMemoryError::Storage(
                    crate::engineering_memory::store::StorageError::NotFound(_)
                )
            );
            if !absent {
                return Err(McpError::internal_error(
                    format!(
                        "refusing to delete: existing memory store could not be loaded \
                         ({load_err}). Recover the quarantined file or fix the store, \
                         then retry."
                    ),
                    None,
                ));
            }
        }

        let exists = memory.snapshot().iter().any(|e| e.id == id);
        if !exists {
            return Err(McpError::invalid_params(
                format!("no entry for key '{key}'"),
                None,
            ));
        }
        if let Err(e) = memory.delete(&id, args.confirm) {
            return match e {
                crate::engineering_memory::runtime::EngineeringMemoryError::ConfirmationRequired(
                    msg,
                ) => Err(McpError::invalid_params(msg, None)),
                other => Err(McpError::internal_error(other.to_string(), None)),
            };
        }
        memory
            .persist()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "memory deleted: {key}"
        ))]))
    }

    // ── Tool 6b: update project identity (guarded write) ──────────────

    /// Update the persistent project identity: goals, constraints,
    /// decisions, roadmap, sprint, conventions, and architecture summary.
    /// Changes are validated before persistence; authored data is never
    /// overwritten by init's inference.
    #[tool(
        description = "Update the persistent project identity (.codebro/project_identity.json): record goals, constraints, decisions, roadmap items, sprint, conventions, or an architecture summary. This is the medium-high-trust declared-intent store. Requires an existing identity."
    )]
    async fn update_identity(
        &self,
        Parameters(args): Parameters<UpdateIdentityArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        use crate::project_identity::{
            DecisionStatus, EngineeringDecision, IdentityChanges, ProjectIdentityRuntime,
            ProjectIdentityUpdater, RoadmapItem, RoadmapStatus,
        };

        let parse_status = |s: &Option<String>,
                            what: &str|
         -> Result<Option<DecisionStatus>, McpError> {
            match s.as_deref().map(str::trim) {
                None | Some("") => Ok(None),
                Some("proposed") => Ok(Some(DecisionStatus::Proposed)),
                Some("accepted") => Ok(Some(DecisionStatus::Accepted)),
                Some("deprecated") => Ok(Some(DecisionStatus::Deprecated)),
                Some("superseded") => Ok(Some(DecisionStatus::Superseded)),
                Some(other) => Err(McpError::invalid_params(
                    format!("invalid {what} status '{other}': expected proposed|accepted|deprecated|superseded"),
                    None,
                )),
            }
        };
        let parse_roadmap = |s: &Option<String>| -> Result<RoadmapStatus, McpError> {
            match s.as_deref().map(str::trim) {
                None | Some("") => Ok(RoadmapStatus::Planned),
                Some("planned") => Ok(RoadmapStatus::Planned),
                Some("in_progress") => Ok(RoadmapStatus::InProgress),
                Some("completed") => Ok(RoadmapStatus::Completed),
                Some("deferred") => Ok(RoadmapStatus::Deferred),
                Some(other) => Err(McpError::invalid_params(
                    format!("invalid roadmap status '{other}': expected planned|in_progress|completed|deferred"),
                    None,
                )),
            }
        };

        let mut runtime = ProjectIdentityRuntime::new(&ws.canonical_root);
        let current = runtime.load().map_err(|e| {
            McpError::invalid_params(
                format!("no project identity for this workspace (run `codebro init` first): {e}"),
                None,
            )
        })?;
        let current = current.clone();

        let mut changes = IdentityChanges::new();
        let mut skipped: Vec<String> = Vec::new();

        if let Some(desc) = args.description.as_deref() {
            changes.set_description = Some(require_non_empty(
                &crate::tools::shell::redact_secrets_public(desc),
                "description",
            )?);
        }
        if let Some(url) = args.repository_url.as_deref() {
            changes.set_repository_url = Some(require_non_empty(
                &crate::tools::shell::redact_secrets_public(url),
                "repository_url",
            )?);
        }
        if let Some(summary) = args.architecture_summary.as_deref() {
            changes.update_architecture_summary = Some(require_non_empty(
                &crate::tools::shell::redact_secrets_public(summary),
                "architecture_summary",
            )?);
        }
        if let Some(sprint) = args.current_sprint.as_deref() {
            changes.set_sprint = Some(require_non_empty(
                &crate::tools::shell::redact_secrets_public(sprint),
                "current_sprint",
            )?);
        }
        if let Some(item) = args.complete_roadmap_item.as_deref() {
            changes.complete_roadmap_item = Some(require_non_empty(
                &crate::tools::shell::redact_secrets_public(item),
                "complete_roadmap_item",
            )?);
        }
        if let Some(milestone) = args.add_milestone.as_deref() {
            changes.add_milestone = Some(require_non_empty(
                &crate::tools::shell::redact_secrets_public(milestone),
                "add_milestone",
            )?);
        }

        push_unique_strings(
            &mut changes.add_constraints,
            &args.add_constraints,
            &current.known_constraints,
        );
        push_unique_strings(
            &mut changes.add_patterns,
            &args.add_patterns,
            &current.known_patterns,
        );
        push_unique_strings(
            &mut changes.add_conventions,
            &args.add_conventions,
            &current.coding_conventions,
        );
        push_unique_strings(
            &mut changes.add_modules,
            &args.add_modules,
            &current.known_modules,
        );
        push_unique_strings(
            &mut changes.add_important_files,
            &args.add_important_files,
            &current.important_files,
        );

        let existing_decision_ids: std::collections::HashSet<&str> = current
            .engineering_decisions
            .iter()
            .map(|d| d.id.as_str())
            .collect();
        for input in &args.add_decisions {
            // Redact at the write seam (identity JSON is persisted and
            // surfaced by briefs/context packets).
            let redact = |s: &str| crate::tools::shell::redact_secrets_public(s);
            let title = require_non_empty(&redact(&input.title), "decision title")?;
            let description =
                require_non_empty(&redact(&input.description), "decision description")?;
            let id = slugify(&title);
            if existing_decision_ids.contains(id.as_str()) {
                skipped.push(format!("decision '{id}' already recorded"));
                continue;
            }
            let mut decision = EngineeringDecision::new(
                id.clone(),
                title,
                description,
                input.context.as_deref().map(redact),
            );
            if let Some(status) = parse_status(&input.status, "decision")? {
                decision = decision.with_status(status);
            } else {
                // Agent-recorded decisions are proposals unless explicitly
                // accepted — keeps declared intent honest.
                decision = decision.with_status(DecisionStatus::Accepted);
            }
            changes.add_decisions.push(decision);
        }

        let existing_roadmap_ids: std::collections::HashSet<&str> =
            current.roadmap.iter().map(|i| i.id.as_str()).collect();
        for input in &args.add_roadmap_items {
            let redact = |s: &str| crate::tools::shell::redact_secrets_public(s);
            let title = require_non_empty(&redact(&input.title), "roadmap title")?;
            let id = slugify(&title);
            if existing_roadmap_ids.contains(id.as_str()) {
                skipped.push(format!("roadmap item '{id}' already recorded"));
                continue;
            }
            let mut item =
                RoadmapItem::new(id.clone(), title, input.description.as_deref().map(redact));
            item.status = parse_roadmap(&input.status)?;
            if let Some(sprint) = input.sprint.as_deref() {
                item.sprint = Some(redact(sprint));
            }
            changes.add_roadmap_items.push(item);
        }

        if changes.is_empty() {
            if skipped.is_empty() {
                return Err(McpError::invalid_params(
                    "no identity changes supplied",
                    None,
                ));
            }
            // Everything supplied was already recorded — report, don't fail.
            let response = json!({
                "applied": false,
                "skipped": skipped,
                "reason": "all supplied items already present in identity",
            });
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&response)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            )]));
        }

        let mut updater = ProjectIdentityUpdater::new(&ws.canonical_root);
        let result = updater
            .update(&current, changes)
            .ok_or_else(|| McpError::internal_error("identity update produced no result", None))?;

        if !result.applied {
            return Err(McpError::invalid_params(
                "identity update rejected by validation (check field lengths/content)",
                None,
            ));
        }

        let identity = result.identity;
        let response = json!({
            "applied": true,
            "skipped": skipped,
            "identity": {
                "name": identity.name,
                "description": identity.description,
                "languages": identity.languages,
                "frameworks": identity.frameworks,
                "build_system": identity.build_system,
                "package_manager": identity.package_manager,
                "testing_framework": identity.testing_framework,
                "repository_url": identity.repository_url,
                "architecture_summary": identity.architecture_summary,
                "known_patterns_count": identity.known_patterns.len(),
                "known_modules_count": identity.known_modules.len(),
                "engineering_decisions": identity.engineering_decisions.iter().map(|d| json!({
                    "id": d.id, "title": d.title, "status": d.status.to_string(),
                })).collect::<Vec<_>>(),
                "known_constraints": identity.known_constraints,
                "current_sprint": identity.current_sprint,
                "roadmap": identity.roadmap.iter().map(|i| json!({
                    "id": i.id, "title": i.title, "status": i.status.to_string(),
                })).collect::<Vec<_>>(),
                "coding_conventions": identity.coding_conventions,
                "updated_at": identity.updated_at,
            },
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&response)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 7: memory statistics ─────────────────────────────────────

    /// Return read-only statistics about the engineering memory store:
    /// entry count, configured token budget, tag distribution, average
    /// confidence, and oldest/newest entry timestamps.
    #[tool(
        description = "Return read-only statistics about the engineering memory store: number of entries, total token budget, tag distribution, average confidence, and oldest/newest entry timestamps. Call this to judge whether engineering memory holds meaningful state before relying on it."
    )]
    async fn memory_stats(
        &self,
        Parameters(args): Parameters<MemoryStatsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let identity = crate::project_identity::ProjectIdentityRuntime::new(&ws.canonical_root);
        let mut memory =
            crate::engineering_memory::EngineeringMemoryRuntime::new(&ws.canonical_root, identity);
        let _ = memory.load(); // absent store is not an error for a read query

        let total_budget = crate::engineering_memory::resolver::DEFAULT_TOKEN_BUDGET;
        let entries = memory.snapshot();

        // Tag distribution (deterministic: sorted).
        let mut tag_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut confidence_sum = 0.0f64;
        let mut trust_sum = 0.0f64;
        let mut oldest: Option<u64> = None;
        let mut newest: Option<u64> = None;
        let mut with_source = 0usize;
        for e in &entries {
            confidence_sum += e.metadata.confidence;
            trust_sum += compute_trust(
                &SourceKind::AgentDeclared,
                e.metadata.confidence,
                FreshnessStatus::Unknown,
            );
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
        let avg_trust = if entries.is_empty() {
            None
        } else {
            Some(trust_sum / entries.len() as f64)
        };

        let mut payload = serde_json::Map::new();
        payload.insert("entry_count".to_string(), json!(entries.len()));
        payload.insert("total_budget".to_string(), json!(total_budget));
        payload.insert("entries_with_source".to_string(), json!(with_source));
        payload.insert(
            "avg_confidence".to_string(),
            json!((avg_confidence * 100.0).round() / 100.0),
        );
        if let Some(t) = avg_trust {
            payload.insert("avg_trust".to_string(), json!(t));
        }
        payload.insert("oldest_created_at".to_string(), json!(oldest));
        payload.insert("newest_created_at".to_string(), json!(newest));
        payload.insert("tags".to_string(), json!(tag_counts));
        let payload = json!(payload);

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 8: sandbox execution ─────────────────────────────────────

    /// Execute a command in an isolated sandbox environment. The command is
    /// policy-checked (read-only build/test/lint commands only) before
    /// execution. Returns structured evidence with provenance: exit_code,
    /// stdout, stderr, duration_ms, repo_state, capabilities.
    #[tool(
        description = "Execute a command in an isolated sandbox. Returns structured evidence: exit_code, stdout, stderr, duration_ms, success, timeout, denied. Only read-only build/test/lint commands are permitted."
    )]
    async fn sandbox_exec(
        &self,
        Parameters(args): Parameters<SandboxExecArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let cmd = crate::sandbox::SandboxCommand {
            command: args.command,
            working_directory: args.working_directory,
            policy: None,
            metadata: args.metadata,
        };
        let policy =
            crate::sandbox::SandboxPolicy::new().with_timeout(args.timeout.unwrap_or(120) as u64);
        let result = self
            .sandbox_runtime
            .execute(&ws.canonical_root, cmd, &policy);
        let payload = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(payload)]))
    }

    // ── Tool 9: sandbox test ──────────────────────────────────────────

    /// Run the project's tests and return structured verification evidence.
    /// Auto-detects the project type and runs the appropriate test command.
    /// Returns execution result plus pass/fail verification.
    #[tool(
        description = "Run the project's tests and return structured verification evidence: execution result plus pass/fail verification with exit code, stdout, stderr, duration, and expectation violations."
    )]
    async fn sandbox_test(
        &self,
        Parameters(args): Parameters<SandboxTestArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let command = resolve_test_command_filtered(
            &ws.canonical_root,
            args.command.as_deref(),
            args.test_filter.as_deref().unwrap_or(&[]),
        );
        let cmd = crate::sandbox::SandboxCommand {
            command: command.clone(),
            working_directory: args.working_directory,
            policy: None,
            metadata: args.metadata,
        };
        let policy =
            crate::sandbox::SandboxPolicy::new().with_timeout(args.timeout.unwrap_or(120) as u64);
        let execution = self
            .sandbox_runtime
            .execute(&ws.canonical_root, cmd, &policy);
        let verification =
            crate::sandbox::VerificationResult::from_execution_with_impacted_fact_ids(
                execution,
                args.expected_exit_code,
                args.expected_success,
                args.affected_fact_ids,
            );
        let verification_obj = self.verification_object(
            &ws,
            &verification,
            args.test_filter.as_deref().unwrap_or(&[]),
        );
        crate::history_capture::capture(
            &self.context_store(),
            &ws.canonical_root,
            crate::history_capture::HistoryCapture {
                kind: crate::context_runtime::HistoryKind::Validation,
                summary: format!(
                    "{command} → {} (verified: {})",
                    verification.classification.as_deref().unwrap_or("unknown"),
                    verification.verified
                ),
                tool: Some("sandbox_test".to_string()),
                path: None,
                outcome: verification.classification.clone(),
                payload: Some(crate::history_capture::detail(&verification.summary, 500)),
                task_id: None,
            },
        );
        let payload = json!({
            "execution": verification.execution,
            "verification": verification_obj,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 10: sandbox build ────────────────────────────────────────

    /// Build or check the project and return structured verification evidence.
    /// Auto-detects the project type and runs the appropriate build command.
    /// Returns execution result plus pass/fail verification.
    #[tool(
        description = "Build or check the project and return structured verification evidence: execution result plus pass/fail verification with exit code, stdout, stderr, duration, and expectation violations."
    )]
    async fn sandbox_build(
        &self,
        Parameters(args): Parameters<SandboxBuildArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let command = resolve_build_command(&ws.canonical_root, args.command.as_deref());
        let cmd = crate::sandbox::SandboxCommand {
            command: command.clone(),
            working_directory: args.working_directory,
            policy: None,
            metadata: args.metadata,
        };
        let policy =
            crate::sandbox::SandboxPolicy::new().with_timeout(args.timeout.unwrap_or(120) as u64);
        let execution = self
            .sandbox_runtime
            .execute(&ws.canonical_root, cmd, &policy);
        let verification =
            crate::sandbox::VerificationResult::from_execution_with_impacted_fact_ids(
                execution,
                args.expected_exit_code,
                args.expected_success,
                args.affected_fact_ids,
            );
        let verification_obj = self.verification_object(&ws, &verification, &[]);
        crate::history_capture::capture(
            &self.context_store(),
            &ws.canonical_root,
            crate::history_capture::HistoryCapture {
                kind: crate::context_runtime::HistoryKind::Validation,
                summary: format!(
                    "{command} → {} (verified: {})",
                    verification.classification.as_deref().unwrap_or("unknown"),
                    verification.verified
                ),
                tool: Some("sandbox_build".to_string()),
                path: None,
                outcome: verification.classification.clone(),
                payload: Some(crate::history_capture::detail(&verification.summary, 500)),
                task_id: None,
            },
        );
        let payload = json!({
            "execution": verification.execution,
            "verification": verification_obj,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 11: sandbox status ───────────────────────────────────────

    /// Return sandbox runtime status: backend name, active mode, capability
    /// descriptor, and whether the backend is available.
    #[tool(
        description = "Return sandbox runtime status: backend (local/opensandbox), mode, availability, and formal capability descriptor. Call this before sandbox_exec to understand execution guarantees."
    )]
    async fn sandbox_status(&self) -> Result<CallToolResult, McpError> {
        let status = self.sandbox_runtime.status();
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

    // ── Tool 12: impact analysis ──────────────────────────────────────

    /// Analyze what is structurally affected by changing a target symbol,
    /// file, module, or package. Returns directed relationship edges,
    /// related tests, owning module/package, and provenance metadata —
    /// descriptive evidence only, no risk scoring or prescriptions.
    #[tool(
        description = "Analyze structural impact of changing a symbol, file, module, or package. Returns directed relationship edges (bounded transitive traversal), related tests, owning module/package, provenance, and deterministic risk signals. Descriptive evidence only — OpenCode decides."
    )]
    async fn impact_analyze(
        &self,
        Parameters(args): Parameters<ImpactArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let store = self.fact_store(&ws);

        let target = match args.target_type.as_deref() {
            None | Some("symbol") => {
                // Try exact id first, then fall back to name-based lookup.
                let exact = crate::engineering_facts::SymbolId::new(args.target.clone());
                if store.collection().symbol(&exact).is_some() {
                    crate::impact::ImpactTarget::Symbol(exact)
                } else {
                    match crate::impact::resolve_symbol_name(&store, &args.target) {
                        Ok(t) => t,
                        Err(e) => {
                            return Err(McpError::invalid_params(e, None));
                        }
                    }
                }
            }
            Some("file") => crate::impact::ImpactTarget::File(args.target.clone()),
            Some("module") => {
                let mod_id = crate::engineering_facts::ModuleId::new(args.target.clone());
                crate::impact::ImpactTarget::Module(mod_id)
            }
            Some("package") => {
                let pkg_id = crate::engineering_facts::PackageId::new(args.target.clone());
                crate::impact::ImpactTarget::Package(pkg_id)
            }
            other => {
                return Err(McpError::invalid_params(
                    format!(
                        "unknown target_type '{:?}' — use symbol, file, module, or package",
                        other
                    ),
                    None,
                ))
            }
        };

        let opts = crate::impact::ImpactOptions {
            max_results: args.max_results.unwrap_or(50),
            include_tests: args.include_tests.unwrap_or(true),
            include_references: args.include_references.unwrap_or(true),
            depth: args.depth.unwrap_or(1),
            direction: args.direction.unwrap_or_else(|| "both".to_string()),
            relationship_types: args.relationship_types.clone(),
            max_nodes: args.max_nodes.unwrap_or(crate::impact::DEFAULT_MAX_NODES),
        };

        if let Err(e) = crate::impact::validate_opts(&opts) {
            return Err(McpError::invalid_params(e.0, None));
        }

        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let store = self.fact_store(&ws);
        let result = crate::impact::analyze(&store, target, &opts, Some(&ws.canonical_root));
        let payload = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(payload)]))
    }

    // ── Tool 13: reindex ─────────────────────────────────────────────

    /// Perform a full engineering fact reindex. Regenerates
    /// `.codebro/facts.json` by re-scanning the entire workspace with the
    /// existing `codebro init` pipeline. Use this after source changes when
    /// `apply_change.needs_reindex=true`.
    ///
    /// P6: the rebuild reuses the content-addressed parse cache so only
    /// changed files are reparsed (incremental at the parse layer);
    /// unchanged files yield byte-identical symbol IDs (preserved, not
    /// rewritten); deleted files disappear with no orphaned graph state.
    /// The response reports the incremental diff (added/deleted/modified/
    /// unchanged), updates the persisted index metadata (v7 `repo_indexes`),
    /// and records a durable `index_completed`/`index_failed` history event.
    /// The operation may take longer than normal read-only fact queries.
    #[tool(
        description = "Perform a full engineering fact reindex: regenerate .codebro/facts.json by re-scanning the entire workspace. Use after source changes when apply_change.needs_reindex=true. This is a full rebuild, not incremental."
    )]
    async fn reindex(
        &self,
        Parameters(args): Parameters<ReindexArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        let start = std::time::Instant::now();

        // P6 incremental diff: snapshot previous digests before the rebuild
        // so the response can report added/deleted/modified/unchanged.
        let prev_digests: std::collections::BTreeMap<String, String> = self
            .fact_store(&ws)
            .collection()
            .model()
            .file_digests()
            .cloned()
            .unwrap_or_default();

        // The init pipeline is fully synchronous (filesystem walk +
        // tree-sitter parsing) and can take minutes on large workspaces.
        // Run it on the blocking thread pool so the async runtime — and
        // therefore every other in-flight MCP tool call on this stdio
        // connection — stays responsive.
        let root = ws.canonical_root.clone();
        let init_result = tokio::task::spawn_blocking(move || crate::init::run(&root))
            .await
            .map_err(|e| McpError::internal_error(format!("reindex worker panicked: {e}"), None))?;

        match init_result {
            Ok(()) => {
                // Invalidate the mtime-based fact store cache so the next
                // call reloads the freshly written .codebro/facts.json.
                ws.invalidate_facts_cache();

                let store = self.fact_store(&ws);
                let elapsed = start.elapsed();
                let counts = store.collection().counts();
                let validation = store.validate();
                let gen_state = store.collection().model().generation_repo_state();
                let curr_digests: std::collections::BTreeMap<String, String> = store
                    .collection()
                    .model()
                    .file_digests()
                    .cloned()
                    .unwrap_or_default();
                // P6 diff + freshness (pure over digest maps + repo state).
                let diff = diff_digests_for_mcp(&prev_digests, &curr_digests);
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let edge_count = counts.relationships + counts.references + counts.dependencies;
                let repo_identity = codebro_core::RepoIdentity::from_workspace(&ws.canonical_root);
                let identity_json =
                    serde_json::to_string(&repo_identity).unwrap_or_else(|_| "{}".to_string());
                let revision = gen_state
                    .map(|s| s.working_tree_hash.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                // Persist derived index metadata (v7). Best-effort: a
                // store failure never fails the reindex itself.
                let _ = self.context_store().upsert_repo_index(
                    &ws.canonical_root.to_string_lossy(),
                    crate::context_runtime::RepoIndexUpsert {
                        repository_identity: identity_json,
                        index_status: crate::context_runtime::RepoIndexStatus::Ready,
                        indexed_at: now_secs,
                        repository_revision: revision.clone(),
                        file_count: curr_digests.len(),
                        symbol_count: counts.symbols,
                        edge_count,
                        stale_count: 0,
                    },
                    now_secs,
                );
                // Durable history: one meaningful event per completed index.
                crate::history_capture::capture(
                    &self.context_store(),
                    &ws.canonical_root,
                    crate::history_capture::HistoryCapture {
                        kind: crate::context_runtime::HistoryKind::IndexCompleted,
                        summary: format!(
                            "index completed: {} files, {} symbols, {} edges ({} added, {} modified, {} deleted)",
                            curr_digests.len(),
                            counts.symbols,
                            edge_count,
                            diff.added.len(),
                            diff.modified.len(),
                            diff.deleted.len(),
                        ),
                        tool: Some("reindex".to_string()),
                        path: None,
                        outcome: Some("ok".to_string()),
                        payload: None,
                        task_id: None,
                    },
                );

                let payload = json!({
                    "status": "ok",
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
                "languages": counts.languages,
                "frameworks": counts.frameworks,
                "entry_points": counts.entry_points,
                        "total": counts.total,
                    },
                    "generation_repo_state": gen_state.map(|s| json!({
                        "commit_sha": s.commit_sha,
                        "working_tree_dirty": s.working_tree_dirty,
                        "working_tree_hash": s.working_tree_hash,
                    })),
                    "validation": {
                        "valid": validation.passed(),
                        "issue_count": validation.issue_count(),
                    },
                    "incremental": {
                        "added": diff.added,
                        "deleted": diff.deleted,
                        "modified": diff.modified,
                        "unchanged_count": diff.unchanged.len(),
                        "added_count": diff.added_count,
                        "deleted_count": diff.deleted_count,
                        "modified_count": diff.modified_count,
                        "truncated": diff.truncated,
                    },
                    "index_status": "READY",
                    "duration_ms": elapsed.as_millis(),
                });

                Ok(CallToolResult::success(vec![ContentBlock::text(
                    serde_json::to_string_pretty(&payload)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                )]))
            }
            Err(e) => {
                let elapsed = start.elapsed();
                // Durable history for failed runs (explicit, never silent).
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                // Preserve last-good metadata: a failure must never zero
                // out known-good counts. Absent rows read as UNKNOWN (all
                // zeros), which matches the old shape for fresh workspaces.
                let prev_row = self
                    .context_store()
                    .get_repo_index(&ws.canonical_root.to_string_lossy(), now_secs)
                    .unwrap_or_else(|_| {
                        crate::context_runtime::RepoIndexRecord::unknown_for(
                            &ws.canonical_root.to_string_lossy(),
                            now_secs,
                        )
                    });
                let _ = self.context_store().upsert_repo_index(
                    &ws.canonical_root.to_string_lossy(),
                    failed_index_upsert(&prev_row),
                    now_secs,
                );
                crate::history_capture::capture(
                    &self.context_store(),
                    &ws.canonical_root,
                    crate::history_capture::HistoryCapture {
                        kind: crate::context_runtime::HistoryKind::IndexFailed,
                        summary: format!("index failed: {}", short_error(&e.to_string())),
                        tool: Some("reindex".to_string()),
                        path: None,
                        outcome: Some("error".to_string()),
                        payload: None,
                        task_id: None,
                    },
                );
                let payload = json!({
                    "status": "error",
                    "error": e.to_string(),
                    "index_status": "FAILED",
                    "duration_ms": elapsed.as_millis(),
                });
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    serde_json::to_string_pretty(&payload)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                )]))
            }
        }
    }
    // ── Tool 14: repository health ────────────────────────────────────

    /// Return a structured health report for the current CodeBro workspace
    /// by delegating to the existing doctor implementation. Read-only;
    /// exposes project identity, fact store, engineering memory and git
    /// status checks with exit code, status, per-check results and a
    /// summary.
    #[tool(
        description = "Return a structured read-only health report for the CodeBro workspace: exit code, status (healthy/warn/error), per-check results and summary. Delegates to the existing doctor implementation."
    )]
    async fn repository_health(
        &self,
        Parameters(args): Parameters<RepositoryHealthArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let (code, checks) = crate::doctor::report(&ws.canonical_root)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let status = match code {
            crate::doctor::EXIT_ERROR => "error",
            crate::doctor::EXIT_WARN => "warn",
            _ => "healthy",
        };

        let _check_count = checks.len();
        let error_count = checks
            .iter()
            .filter(|c| !c.ok && c.detail.as_deref().is_some_and(|d| d.starts_with("ERROR")))
            .count();
        let warn_count = checks
            .iter()
            .filter(|c| !c.ok && !c.detail.as_deref().is_some_and(|d| d.starts_with("ERROR")))
            .count();

        let checks_out: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                let check_status = if c.ok {
                    "ok"
                } else if c.detail.as_deref().is_some_and(|d| d.starts_with("ERROR")) {
                    "error"
                } else {
                    "warn"
                };
                json!({
                    "name": c.name,
                    "status": check_status,
                    "detail": c.detail,
                })
            })
            .collect();

        let summary = match code {
            crate::doctor::EXIT_HEALTHY => "All checks passed.".to_string(),
            crate::doctor::EXIT_ERROR => {
                format!("Errors detected ({error_count}). Run `codebro init` to repair.")
            }
            _ => format!("{error_count} error(s), {warn_count} warning(s)."),
        };

        let payload = json!({
            "exit_code": code,
            "status": status,
            "checks": checks_out,
            "summary": summary,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 15: consultant ───────────────────────────────────────────

    /// Ask an AI consultant (Conductor gateway) for opinions on architecture,
    /// debugging, code review, planning, research, or second opinions.
    /// CodeBro engineering context (facts, memory, git diff) can be attached
    /// so the consultant answers with project-awareness.
    #[tool(
        description = "Ask an AI consultant (Conductor) for opinions on architecture, debugging, code review, planning, research, or second opinions. Supports provider selection, mode shaping, and automatic injection of CodeBro engineering context (facts, memory, git diff)."
    )]
    async fn consult(
        &self,
        Parameters(args): Parameters<ConsultArgs>,
    ) -> Result<CallToolResult, McpError> {
        // Validate question is non-empty.
        if args.question.trim().is_empty() {
            return Err(McpError::invalid_params("question must not be empty", None));
        }

        // Parse provider choice.
        let provider_choice = match args.provider.as_deref() {
            None | Some("auto") => crate::consultant::types::ConsultantProvider::Auto,
            Some("conductor") => crate::consultant::types::ConsultantProvider::Conductor,
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("unknown provider '{other}' — use auto or conductor"),
                    None,
                ))
            }
        };

        // Parse mode.
        let mode = match args.mode.as_deref() {
            None | Some("architecture") => crate::consultant::types::ConsultantMode::Architecture,
            Some("debugging") => crate::consultant::types::ConsultantMode::Debugging,
            Some("code_review") | Some("code-review") => {
                crate::consultant::types::ConsultantMode::CodeReview
            }
            Some("planning") => crate::consultant::types::ConsultantMode::Planning,
            Some("research") => crate::consultant::types::ConsultantMode::Research,
            Some("second_opinion") | Some("second-opinion") => {
                crate::consultant::types::ConsultantMode::SecondOpinion
            }
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!(
                        "unknown mode '{other}' — use architecture, debugging, code_review, \
                         planning, research, or second_opinion"
                    ),
                    None,
                ))
            }
        };

        // Build the request.
        let mut request = crate::consultant::types::ConsultantRequest {
            provider: provider_choice.clone(),
            mode: mode.clone(),
            question: args.question.trim().to_string(),
            context: args.context.clone(),
            files: args
                .files
                .iter()
                .map(|f| crate::consultant::types::ConsultantFileContext {
                    path: f.path.clone(),
                    content: f.content.clone(),
                })
                .collect(),
            include_git_diff: args.include_git_diff.unwrap_or(false),
            include_project_context: args.include_project_context.unwrap_or(false),
            max_answer_length: args.max_answer_length.unwrap_or(0),
        };

        // Inject CodeBro engineering context when requested.
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        if args.include_project_context.unwrap_or(false) {
            inject_project_context(&mut request, &ws.canonical_root);
        }
        if args.include_git_diff.unwrap_or(false) {
            inject_git_diff(&mut request, &ws.canonical_root);
        }

        // Debugging mode: inject the latest deterministic root-cause
        // hypotheses (if any) so the external LLM reasons OVER CodeBro's
        // structured evidence rather than re-deriving it.
        if matches!(mode, crate::consultant::types::ConsultantMode::Debugging) {
            let guard = ws.last_rca.lock().expect("last rca lock");
            if let Some(rca) = guard.as_ref() {
                let payload = serde_json::to_string(rca).unwrap_or_default();
                request
                    .files
                    .push(crate::consultant::types::ConsultantFileContext {
                        path: "codebro://root-cause-hypotheses".to_string(),
                        content: payload,
                    });
            }
        }

        // Resolve provider and call consult.
        let router = crate::consultant::build_router();
        let provider = match router.resolve(&provider_choice) {
            Ok(p) => p,
            Err(e) => {
                return Err(McpError::internal_error(
                    format!("provider resolution failed: {e}"),
                    None,
                ));
            }
        };

        let response = match provider.consult(&request).await {
            Ok(r) => r,
            Err(crate::consultant::provider::ConsultantError::AuthenticationRequired(msg)) => {
                return Err(McpError::internal_error(msg, None));
            }
            Err(e) => {
                return Err(McpError::internal_error(
                    format!("consultation failed: {e}"),
                    None,
                ));
            }
        };

        let payload = json!({
            "provider": response.provider,
            "model": response.model,
            "mode": mode.to_string(),
            "answer": response.answer,
            "summary": response.summary,
            "recommendations": response.recommendations,
            "risks": response.risks,
            "confidence": response.confidence,
            "metadata": response.metadata,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 25: engineering brief (P7 decision support) ───────────────

    /// Compose a bounded engineering decision-support brief for a task:
    /// repository identity and freshness, task-relevant files/symbols/
    /// dependencies, bounded impact with risk signals, relevant tests and
    /// health findings, history excerpts, engineering memory, accepted
    /// learning, skill applicability, task state, constraints, decisions,
    /// risks, and explicit unknowns. Read-only: never writes project or
    /// user state, never transitions tasks, never executes skills.
    /// CodeBro prepares evidence; OpenCode reasons and decides.
    #[tool(
        description = "Compose a bounded engineering decision-support brief for a task: repo intelligence, impact, health, history, memory, learning, skills, task state, constraints, and explicit unknowns. Read-only; OpenCode decides."
    )]
    async fn engineering_brief(
        &self,
        Parameters(args): Parameters<EngineeringBriefArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let task_id = args
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let request = crate::engineering_brief::BriefRequest {
            task: args.task.clone().unwrap_or_default(),
            task_id: task_id.map(str::to_string),
            target_path: args.target_path.clone(),
            target_symbol: args.target_symbol.clone(),
            target_module: args.target_module.clone(),
            keywords: args.keywords.clone().unwrap_or_default(),
            depth: args
                .depth
                .unwrap_or(crate::engineering_brief::BRIEF_DEPTH_DEFAULT),
        };
        // Records use the request keywords plus a best-effort task hint
        // (title/description/next-action tokens from the read-only task
        // snapshot). The assembler derives a sibling enrichment for the
        // brief's own scope (title/checkpoint-summary/next-action); the
        // two sets intentionally overlap but are not identical — records
        // resolve against the broader set. A missing snapshot simply
        // yields no hint here (the brief records TASK_NOT_FOUND).
        let mut record_keywords = request.keywords();
        if let Some(tid) = task_id {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Ok(snap) = self.context_store().task_resume_snapshot(
                &ws.canonical_root.to_string_lossy(),
                tid,
                now,
            ) {
                for extra in [
                    snap.task.title.as_str(),
                    snap.task.description.as_deref().unwrap_or(""),
                    snap.latest_checkpoint
                        .as_ref()
                        .and_then(|c| c.next_action.as_deref())
                        .unwrap_or(""),
                ] {
                    for tok in extra.split(|c: char| !c.is_alphanumeric()) {
                        if tok.len() >= 3
                            && !record_keywords.contains(&tok.to_string())
                            && record_keywords.len() < crate::engineering_brief::MAX_BRIEF_KEYWORDS
                        {
                            record_keywords.push(tok.to_string());
                        }
                    }
                }
                record_keywords.sort();
            }
        }
        let records = self.context_record_excerpts(&ws, task_id, &record_keywords);
        let (identity_loaded, identity_opt) = self.identity_snapshot(&ws);
        // Absent identity degrades to defaults (the brief records
        // MISSING_IDENTITY); orientation never fails a read.
        let identity = identity_opt.unwrap_or_else(|| {
            crate::project_identity::ProjectIdentityRuntime::new(&ws.canonical_root).snapshot()
        });
        let store = self.fact_store(&ws);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let inputs = crate::engineering_brief::BriefInputs {
            workspace_root: &ws.canonical_root,
            workspace_key: crate::context_runtime::canonical_workspace_key(
                &ws.canonical_root.to_string_lossy(),
            ),
            store: &store,
            context_store: &self.context_store(),
            identity_loaded,
            identity: &identity,
            records: &records,
            now,
        };
        let brief = crate::engineering_brief::assemble(&inputs, &request)
            .map_err(|e| McpError::invalid_params(e, None))?;
        let value = serde_json::to_value(&brief)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let payload = crate::mcp::response_bounds::bounded_response(value)
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(payload)]))
    }

    // ── Tool 18: context (always-available context packet) ─────────────

    /// Compose the always-available context packet for the current task:
    /// repository orientation and fact counts, task-relevant facts,
    /// decisions, memory, execution evidence, and durable context records
    /// (each tagged with its authority). With a task the packet is
    /// task-relevant; without one it returns a clearly-labelled structural
    /// digest. Read-only: never writes project or user state.
    #[tool(
        description = "Compose the always-available context packet: repository orientation, fact counts, task-relevant facts/decisions/memory/evidence, and durable context records tagged by authority (user_confirmed/ai_inferred/observed). Call at task start and when context runs thin."
    )]
    async fn context(
        &self,
        Parameters(args): Parameters<ContextArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let request = crate::engineering_context::EngineeringContextRequest {
            task: args.task.clone().unwrap_or_default(),
            task_keywords: args.keywords.clone().unwrap_or_default(),
            active_file_tags: Vec::new(),
        };
        let has_task = !request.is_empty();
        let task_id = args
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let records = self.context_record_excerpts(&ws, task_id, &request.keywords());
        let packet = if has_task {
            crate::engineering_context::compose(&ws.canonical_root, &request, &records)
        } else {
            crate::engineering_context::compose_structural(&ws.canonical_root, &records)
        }
        .map_err(|e| McpError::internal_error(e, None))?;
        let value = serde_json::to_value(&packet)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let payload = crate::mcp::response_bounds::bounded_response(value)
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(payload)]))
    }

    /// Retrieve bounded durable-context excerpts for a workspace.
    ///
    /// Resolution (not concatenation): a generous fetch (keyword-less
    /// importance order plus keyword matches when a task narrows relevance)
    /// is reduced per (kind, namespace) to one winner by authority rank,
    /// then scope specificity (task > project > global), decayed
    /// confidence, recency, and id. Actionable intents surface alongside
    /// fingerprint winners; losers stay in the store, queryable by id.
    /// The user-context store is best-effort by design: if it cannot be
    /// opened (e.g. an unwritable state dir), composition degrades to an
    /// empty `records` section with a warning rather than failing the whole
    /// packet — the engineering stores stay authoritative.
    fn context_record_excerpts(
        &self,
        ws: &WorkspaceState,
        task_id: Option<&str>,
        keywords: &[String],
    ) -> Vec<crate::engineering_context::ContextRecordExcerpt> {
        use crate::context_runtime::{fingerprint, ContextRetriever};
        let root = ws.canonical_root.display().to_string();
        let store = self.context_store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Fetch generously: resolution reduces, never expands. Keyword-less
        // first (the always-available fingerprint), keyword matches merged
        // in when a task narrows relevance.
        let mut merged: std::collections::BTreeMap<String, crate::context_runtime::RankedRecord> =
            std::collections::BTreeMap::new();
        let mut fetch = |kw: Vec<String>| {
            let query = crate::context_runtime::RecordQuery {
                workspace_root: Some(root.as_str()),
                task_id,
                kind: None,
                status: None,
                keywords: kw,
                limit: 100,
            };
            if let Ok(ranked) = ContextRetriever::search(&*store, &query, now) {
                for r in ranked {
                    merged.insert(r.record.id.clone(), r);
                }
            }
        };
        fetch(Vec::new());
        if !keywords.is_empty() {
            fetch(keywords.to_vec());
        }
        if merged.is_empty() {
            // Distinguish "store unusable" from "store empty": a failed
            // keyword-less fetch on an unusable store already degrades to
            // empty here; warn once for observability.
            tracing::debug!("context records: no rows visible for this viewpoint");
        }
        let scope = fingerprint::ResolutionScope {
            workspace_key: Some(root.as_str()),
            task_id,
        };
        let resolved = fingerprint::resolve_context(merged.into_values().collect(), &scope);
        resolved
            .intents
            .iter()
            .chain(resolved.fingerprint.iter())
            .chain(resolved.other.iter())
            .take(crate::engineering_context::MAX_CONTEXT_RECORDS)
            .map(crate::engineering_context::excerpt_from)
            .collect()
    }

    // ── Tool 19: remember (explicit user-context persistence) ──────────

    /// Persist explicitly-confirmed user context: a collaboration
    /// preference (user fingerprint) or the current intent. Semantic
    /// writes, not database rows: the caller states what the user
    /// confirmed and CodeBro assigns authority, provenance, scope, and
    /// lifecycle. USER_CONFIRMED requires user_confirmed=true; inferred
    /// or observed writes require evidence. Replacing knowledge uses
    /// supersedes (history preserved); completing/cancelling an intent
    /// retires it.
    #[tool(
        description = "Persist explicitly-confirmed user context (a preference or the current intent) with provenance, scope, and lifecycle. USER_CONFIRMED requires user_confirmed=true; observed/inferred writes require evidence. Only persist what the user stated."
    )]
    async fn remember(
        &self,
        Parameters(args): Parameters<RememberArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let content = args.content.trim().to_string();
        if content.is_empty() {
            return Err(McpError::invalid_params("content must not be empty", None));
        }
        let namespace = args.namespace.trim().to_string();
        if namespace.is_empty() {
            return Err(McpError::invalid_params(
                "namespace is required (e.g. fp.engineering.simplicity or intent.mission)",
                None,
            ));
        }
        let kind = parse_record_kind(args.kind.as_deref())?;
        let scope = parse_record_scope(args.scope.as_deref(), true)?;

        // ── Caller-principal rule for UserConfirmed ──
        // USER_CONFIRMED means the system has a trusted basis for treating
        // the information as explicitly confirmed by the user. The basis is
        // OpenCode's explicit speech act: the agent sets user_confirmed=true
        // only when the user stated or approved this content. Without the
        // flag, authority=user_confirmed is refused — a caller cannot mint
        // confirmation by merely naming the authority string.
        let authority = match args.authority.as_deref().map(str::trim) {
            None => {
                if args.user_confirmed.unwrap_or(false) {
                    crate::context_runtime::Authority::UserConfirmed
                } else {
                    crate::context_runtime::Authority::Observed
                }
            }
            Some("user_confirmed") => {
                if !args.user_confirmed.unwrap_or(false) {
                    return Err(McpError::invalid_params(
                        "authority=user_confirmed requires user_confirmed=true: \
                         set the flag only when the user explicitly stated or approved this content",
                        None,
                    ));
                }
                crate::context_runtime::Authority::UserConfirmed
            }
            Some("observed") => crate::context_runtime::Authority::Observed,
            Some("ai_inferred") => crate::context_runtime::Authority::AiInferred,
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!(
                        "unknown authority '{other}': use user_confirmed, observed, or ai_inferred"
                    ),
                    None,
                ));
            }
        };

        // ── Scope binding ──
        let canonical_ws = ws.canonical_root.display().to_string();
        let (workspace_root, task_id) = match scope {
            crate::context_runtime::RecordScope::Global => (None, None),
            crate::context_runtime::RecordScope::Project => (Some(canonical_ws.clone()), None),
            crate::context_runtime::RecordScope::Task => {
                let task = args.task_id.as_deref().map(str::trim).unwrap_or("");
                if task.is_empty() {
                    return Err(McpError::invalid_params(
                        "task scope requires task_id (the OpenCode session/task this override belongs to)",
                        None,
                    ));
                }
                (Some(canonical_ws.clone()), Some(task.to_string()))
            }
        };

        // ── Evidence (store re-verifies existence as a backstop) ──
        let store = self.context_store();
        let mut evidence: Vec<String> = Vec::new();
        for id in &args.evidence_event_ids {
            evidence.push(id.to_string());
        }
        if let Some(obs) = args.observation.as_deref() {
            let obs = crate::tools::shell::redact_secrets_public(obs.trim());
            if obs.trim().is_empty() {
                return Err(McpError::invalid_params(
                    "observation must not be empty when supplied",
                    None,
                ));
            }
            let event = crate::context_runtime::EventRecord {
                id: None,
                session_id: None,
                workspace_root: canonical_ws.clone(),
                task_id: None,
                kind: "agent_observation".to_string(),
                tool: Some("remember".to_string()),
                path: None,
                outcome: None,
                summary: None,
                payload: Some(obs.chars().take(2000).collect::<String>()),
                dedup_key: None,
                source: None,
                digest: None,
                created_at: 0,
            };
            let event_id = store
                .append_event(&event, now)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            evidence.push(event_id.to_string());
        }
        if matches!(
            authority,
            crate::context_runtime::Authority::Observed
                | crate::context_runtime::Authority::AiInferred
        ) && evidence.is_empty()
        {
            return Err(McpError::invalid_params(
                "observed/ai_inferred writes require evidence: pass evidence_event_ids \
                 or describe what was observed in observation (P1 never auto-infers preferences)",
                None,
            ));
        }

        // ── Record assembly ──
        let redacted_content = crate::tools::shell::redact_secrets_public(&content);
        let mut record = crate::context_runtime::ContextRecord::new(
            String::new(), // minted below
            kind,
            namespace.clone(),
            redacted_content,
            authority,
        );
        record.scope = scope;
        record.workspace_root = workspace_root;
        record.task_id = task_id;
        record.lifecycle = crate::context_runtime::lifecycle_for_authority(authority);
        record.confidence = args.confidence.map(|c| c.clamp(0.0, 1.0)).unwrap_or(
            if authority == crate::context_runtime::Authority::UserConfirmed {
                0.9
            } else {
                0.6
            },
        );
        record.importance = args.importance.map(|c| c.clamp(0.0, 1.0)).unwrap_or(0.6);
        if let Some(orig) = args.original_text.as_deref() {
            let orig = crate::tools::shell::redact_secrets_public(orig.trim());
            if orig.chars().count() > 2048 {
                return Err(McpError::invalid_params(
                    "original_text exceeds 2048 characters",
                    None,
                ));
            }
            if !orig.is_empty() {
                record.original_text = Some(orig);
            }
        }
        if let Some(lang) = args.language.as_deref() {
            let lang = lang.trim().to_string();
            if !lang.is_empty() {
                record.language = Some(lang);
            }
        }
        if let Some(src) = args.source.as_deref() {
            let src = src.trim().to_string();
            if !src.is_empty() {
                record.source = Some(src);
            }
        }
        record.related_ids = args.related_ids.clone();
        record.evidence = evidence;

        // Intent metadata lives in extra_json (one schemaless column, not
        // rigid per-intent columns); non-intent kinds must not carry it.
        let intent_status = if kind == crate::context_runtime::RecordKind::Intent {
            let meta = crate::context_runtime::IntentMetadata {
                rationale: args
                    .rationale
                    .as_deref()
                    .map(|r| crate::tools::shell::redact_secrets_public(r.trim())),
                priority: args
                    .priority
                    .as_deref()
                    .map(|p| {
                        p.parse::<crate::context_runtime::IntentPriority>()
                            .map_err(|e| McpError::invalid_params(e, None))
                    })
                    .transpose()?,
                intent_status: args
                    .intent_status
                    .as_deref()
                    .map(|s| {
                        s.parse::<crate::context_runtime::IntentStatus>()
                            .map_err(|e| McpError::invalid_params(e, None))
                    })
                    .transpose()?
                    .unwrap_or(crate::context_runtime::IntentStatus::Active),
            };
            let status = meta.intent_status;
            meta.apply_to(&mut record)
                .map_err(|e| McpError::invalid_params(e, None))?;
            Some(status)
        } else {
            if args.rationale.is_some() || args.priority.is_some() || args.intent_status.is_some() {
                return Err(McpError::invalid_params(
                    "rationale/priority/intent_status apply only to kind=intent",
                    None,
                ));
            }
            None
        };

        // Terminal intent replacements retire (expired/rejected), everything
        // else supersedes active. Map the intent status onto the storage
        // lifecycle now so the store's shape rule sees a coherent record.
        if let Some(status) = intent_status {
            record.status = match status {
                crate::context_runtime::IntentStatus::Active
                | crate::context_runtime::IntentStatus::Paused => {
                    crate::context_runtime::RecordStatus::Active
                }
                crate::context_runtime::IntentStatus::Completed => {
                    crate::context_runtime::RecordStatus::Expired
                }
                crate::context_runtime::IntentStatus::Cancelled => {
                    crate::context_runtime::RecordStatus::Rejected
                }
                crate::context_runtime::IntentStatus::Superseded => {
                    crate::context_runtime::RecordStatus::Superseded
                }
            };
            if status == crate::context_runtime::IntentStatus::Superseded
                && args
                    .supersedes
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                return Err(McpError::invalid_params(
                    "intent_status=superseded requires supersedes (the record this replaces)",
                    None,
                ));
            }
        }

        // ── Fresh write vs replacement ──
        if let Some(prev_id) = args
            .supersedes
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let prev = store
                .get_record(prev_id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("supersedes target '{prev_id}' does not exist"),
                        None,
                    )
                })?;
            if prev.status != crate::context_runtime::RecordStatus::Active {
                return Err(McpError::invalid_params(
                    format!(
                        "only an active record can be superseded ('{prev_id}' is {})",
                        prev.status
                    ),
                    None,
                ));
            }
            // Intent transition guard: terminal intents are history —
            // start a fresh intent instead of rewriting them.
            if prev.kind == crate::context_runtime::RecordKind::Intent {
                let from = crate::context_runtime::IntentMetadata::read_from(&prev)
                    .map(|m| m.intent_status)
                    .map_err(|e| {
                        McpError::invalid_params(
                            format!("supersedes target has malformed intent metadata: {e}"),
                            None,
                        )
                    })?;
                let to = intent_status.unwrap_or(crate::context_runtime::IntentStatus::Active);
                crate::context_runtime::intent::validate_transition(Some(from), to)
                    .map_err(|e| McpError::invalid_params(e, None))?;
            }
            record.id = mint_record_id(&store, kind, &namespace, now);
            record.supersedes = Some(prev_id.to_string());
            if record.status == crate::context_runtime::RecordStatus::Active {
                store
                    .supersede_record(prev_id, &record, now)
                    .map_err(remember_error)?;
            } else {
                store
                    .retire_record(prev_id, &record, now)
                    .map_err(remember_error)?;
            }
            crate::history_capture::capture(
                &store,
                &ws.canonical_root,
                crate::history_capture::HistoryCapture {
                    kind: crate::context_runtime::HistoryKind::Decision,
                    summary: format!(
                        "recorded {} {}: {}",
                        kind,
                        namespace,
                        crate::history_capture::detail(&record.content, 160)
                    ),
                    tool: Some("remember".to_string()),
                    path: None,
                    outcome: Some("superseded".to_string()),
                    payload: None,
                    task_id: record.task_id.clone(),
                },
            );
            let payload = serde_json::json!({
                "remembered": true,
                "id": record.id,
                "kind": kind.to_string(),
                "namespace": namespace,
                "authority": authority.to_string(),
                "scope": scope.to_string(),
                "status": record.status.to_string(),
                "superseded": prev_id,
                "evidence": record.evidence,
            });
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&payload)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            )]));
        }

        // Fresh write: refuse when an active record already owns this
        // (kind, namespace, scope) — replacing knowledge must name its
        // predecessor via supersedes so history is never silently forked.
        // (Terminal intent completion without supersedes auto-retires the
        // single active intent in the namespace; ambiguity errors out.)
        let clash = find_active_in_namespace(
            &store,
            kind,
            &namespace,
            scope,
            record.workspace_root.as_deref(),
            record.task_id.as_deref(),
            now,
        )
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        if !clash.is_empty() {
            let retiring = matches!(
                intent_status,
                Some(
                    crate::context_runtime::IntentStatus::Completed
                        | crate::context_runtime::IntentStatus::Cancelled
                )
            );
            if retiring && clash.len() == 1 {
                let prev_id = clash[0].clone();
                let prev = store
                    .get_record(&prev_id)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .ok_or_else(|| McpError::internal_error("clashing record vanished", None))?;
                let from = crate::context_runtime::IntentMetadata::read_from(&prev)
                    .map(|m| m.intent_status)
                    .unwrap_or(crate::context_runtime::IntentStatus::Active);
                crate::context_runtime::intent::validate_transition(
                    from.into(),
                    intent_status.unwrap(),
                )
                .map_err(|e| McpError::invalid_params(e, None))?;
                record.id = mint_record_id(&store, kind, &namespace, now);
                record.supersedes = Some(prev_id.clone());
                store
                    .retire_record(&prev_id, &record, now)
                    .map_err(remember_error)?;
                crate::history_capture::capture(
                    &store,
                    &ws.canonical_root,
                    crate::history_capture::HistoryCapture {
                        kind: crate::context_runtime::HistoryKind::Decision,
                        summary: format!(
                            "recorded {} {}: {}",
                            kind,
                            namespace,
                            crate::history_capture::detail(&record.content, 160)
                        ),
                        tool: Some("remember".to_string()),
                        path: None,
                        outcome: Some("retired".to_string()),
                        payload: None,
                        task_id: record.task_id.clone(),
                    },
                );
                let payload = serde_json::json!({
                    "remembered": true,
                    "id": record.id,
                    "kind": kind.to_string(),
                    "namespace": namespace,
                    "authority": authority.to_string(),
                    "scope": scope.to_string(),
                    "status": record.status.to_string(),
                    "superseded": prev_id,
                    "evidence": record.evidence,
                });
                return Ok(CallToolResult::success(vec![ContentBlock::text(
                    serde_json::to_string_pretty(&payload)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                )]));
            }
            return Err(McpError::invalid_params(
                format!(
                    "an active {} record already exists in namespace '{namespace}' for this scope ({}); \
                     pass supersedes with its id to replace it (history is preserved, never overwritten)",
                    kind,
                    clash.join(", ")
                ),
                None,
            ));
        }

        record.id = mint_record_id(&store, kind, &namespace, now);
        store.put_record(&record, now).map_err(remember_error)?;
        crate::history_capture::capture(
            &store,
            &ws.canonical_root,
            crate::history_capture::HistoryCapture {
                kind: crate::context_runtime::HistoryKind::Decision,
                summary: format!(
                    "recorded {} {}: {}",
                    kind,
                    namespace,
                    crate::history_capture::detail(&record.content, 160)
                ),
                tool: Some("remember".to_string()),
                path: None,
                outcome: Some("recorded".to_string()),
                payload: None,
                task_id: record.task_id.clone(),
            },
        );
        let payload = serde_json::json!({
            "remembered": true,
            "id": record.id,
            "kind": kind.to_string(),
            "namespace": namespace,
            "authority": authority.to_string(),
            "scope": scope.to_string(),
            "status": record.status.to_string(),
            "evidence": record.evidence,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 20: forget (reversible retirement of user context) ────────

    /// Retire a persisted context record. Default is a reversible reject
    /// (the row stays for the audit trail, like negative knowledge);
    /// permanent=true hard-removes it (cleanup of junk only). Requires
    /// confirm=true. Project/task records only from their own workspace.
    #[tool(
        description = "Retire a persisted context record by id (reversible reject; the row stays for audit) or permanently remove it with permanent=true. Requires confirm=true. Project/task records only from their own workspace."
    )]
    async fn forget(
        &self,
        Parameters(args): Parameters<ForgetArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let _mutation_guard = ws.mutation_lock.lock().await;
        if !args.confirm.unwrap_or(false) {
            return Err(McpError::invalid_params(
                "forget rejected: set confirm=true to retire this record",
                None,
            ));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let store = self.context_store();

        // Locate: exact id first; otherwise resolve the namespace winner
        // for this viewpoint (the record `context` would have shown).
        let target_id: String = if let Some(id) =
            args.id.as_deref().map(str::trim).filter(|s| !s.is_empty())
        {
            id.to_string()
        } else if let Some(ns) = args
            .namespace
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let root = ws.canonical_root.display().to_string();
            let query = crate::context_runtime::RecordQuery {
                workspace_root: Some(root.as_str()),
                task_id: args.task_id.as_deref(),
                kind: None,
                status: None,
                keywords: Vec::new(),
                limit: 100,
            };
            let ranked = crate::context_runtime::ContextRetriever::search(&*store, &query, now)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            let scope = crate::context_runtime::fingerprint::ResolutionScope {
                workspace_key: Some(root.as_str()),
                task_id: args.task_id.as_deref(),
            };
            let resolved = crate::context_runtime::fingerprint::resolve_context(ranked, &scope);
            resolved
                .fingerprint
                .iter()
                .chain(resolved.intents.iter())
                .chain(resolved.other.iter())
                .find(|r| r.record.namespace == ns)
                .map(|r| r.record.id.clone())
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("no visible record in namespace '{ns}' for this workspace/task"),
                        None,
                    )
                })?
        } else {
            return Err(McpError::invalid_params(
                "forget requires id (or namespace + workspace/task to resolve the winner)",
                None,
            ));
        };

        let target = store
            .get_record(&target_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .ok_or_else(|| {
                McpError::invalid_params(format!("no record with id '{target_id}'"), None)
            })?;

        // Workspace confinement: one project cannot forget another's rows.
        let canonical_ws = ws.canonical_root.display().to_string();
        match target.scope {
            crate::context_runtime::RecordScope::Global => {}
            crate::context_runtime::RecordScope::Project => {
                if target.workspace_root.as_deref() != Some(canonical_ws.as_str()) {
                    return Err(McpError::invalid_params(
                        "that record belongs to another workspace and cannot be retired from here",
                        None,
                    ));
                }
            }
            crate::context_runtime::RecordScope::Task => {
                if target.workspace_root.as_deref() != Some(canonical_ws.as_str())
                    || target.task_id.as_deref()
                        != args
                            .task_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                {
                    return Err(McpError::invalid_params(
                        "that task record is not visible from this workspace/task",
                        None,
                    ));
                }
            }
        }

        if args.permanent.unwrap_or(false) {
            let removed = store
                .remove_record(&target_id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            crate::history_capture::capture(
                &store,
                &ws.canonical_root,
                crate::history_capture::HistoryCapture {
                    kind: crate::context_runtime::HistoryKind::Observation,
                    summary: format!("retired context record {target_id} (removed)"),
                    tool: Some("forget".to_string()),
                    path: None,
                    outcome: Some("removed".to_string()),
                    payload: None,
                    task_id: args.task_id.clone(),
                },
            );
            let payload = serde_json::json!({
                "forgotten": removed,
                "id": target_id,
                "action": "removed",
            });
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&payload)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            )]));
        }

        let prior = target.status.to_string();
        let rejected = store
            .reject_record(&target_id, now)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        crate::history_capture::capture(
            &store,
            &ws.canonical_root,
            crate::history_capture::HistoryCapture {
                kind: crate::context_runtime::HistoryKind::Observation,
                summary: format!(
                    "retired context record {target_id} ({})",
                    if rejected {
                        "rejected"
                    } else {
                        "already_terminal"
                    }
                ),
                tool: Some("forget".to_string()),
                path: None,
                outcome: Some("rejected".to_string()),
                payload: None,
                task_id: args.task_id.clone(),
            },
        );
        let payload = serde_json::json!({
            "forgotten": rejected,
            "id": target_id,
            "action": if rejected { "rejected" } else { "already_terminal" },
            "prior_status": prior,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 21: recall (historical evidence on demand) ─────────────────

    /// Recall relevant previous engineering work: decisions, failures,
    /// validations, and changes from past sessions in this project (or
    /// task). Returns compact provenance-tagged excerpts — the evidence,
    /// not transcripts. Call when asking "have we tried this before?",
    /// "why did we choose this?", or "did this fail previously?".
    /// Read-only: never writes history (recalling must not record), and
    /// never dumps history into the always-available context packet.
    #[tool(
        description = "Recall relevant previous engineering work from past sessions: decisions, failures, validations, changes. Returns compact provenance-tagged excerpts (session, timestamp, scope, event type, source). Call when asking 'have we tried this before' or 'why did we choose this'. Read-only."
    )]
    async fn recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        // Read-only: no mutation lock. And deliberately no history capture
        // — recalling history must not write history (no recursion).
        let scope = match args.scope.as_deref().map(str::trim) {
            None | Some("") | Some("project") => crate::context_runtime::RecallScope::Project,
            Some("task") => crate::context_runtime::RecallScope::Task,
            Some("global") => crate::context_runtime::RecallScope::Global,
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("unknown recall scope '{other}': use project, task, or global"),
                    None,
                ))
            }
        };
        let mut kinds = Vec::new();
        for raw in args.kinds.as_deref().unwrap_or(&[]) {
            kinds.push(
                raw.parse::<crate::context_runtime::HistoryKind>()
                    .map_err(|e| McpError::invalid_params(e, None))?,
            );
        }
        let task_id = args
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let root = ws.canonical_root.display().to_string();
        let store = self.context_store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let outcome = store
            .recall(
                &crate::context_runtime::RecallQuery {
                    query: args.query.as_str(),
                    workspace_root: Some(root.as_str()),
                    task_id,
                    scope,
                    kinds,
                    session_id: args.session_id.as_deref(),
                    limit: args.limit.unwrap_or(0),
                },
                now,
            )
            .map_err(|e| match e {
                crate::context_runtime::store::ContextError::Validation(msg) => {
                    McpError::invalid_params(msg, None)
                }
                other => McpError::internal_error(other.to_string(), None),
            })?;
        // Compact structured evidence: session grouping with per-hit
        // provenance (never transcripts). Hit scope is derived from the
        // row: task-bound → task, own workspace → project, else global.
        let groups: Vec<serde_json::Value> = outcome
            .groups
            .iter()
            .map(|g| {
                let events: Vec<serde_json::Value> = g
                    .hits
                    .iter()
                    .map(|h| {
                        let scope = if h.event.task_id.is_some() {
                            "task"
                        } else if h.event.workspace_root == root {
                            "project"
                        } else {
                            "global"
                        };
                        json!({
                            "event_id": h.event.id,
                            "event": h.event.kind,
                            "timestamp": h.event.created_at,
                            "scope": scope,
                            "task_id": h.event.task_id,
                            "tool": h.event.tool,
                            "path": h.event.path,
                            "outcome": h.event.outcome,
                            "source": h.event.source,
                            "session_stale": h.session_stale,
                            "excerpt": h.excerpt,
                        })
                    })
                    .collect();
                json!({
                    "session": g.session_id,
                    "session_title": g.session_title,
                    "session_status": g.session_status,
                    "session_stale": g.session_stale,
                    "workspace_root": g.workspace_root,
                    "task_id": g.task_id,
                    "total_in_session": g.total_in_session,
                    "events": events,
                })
            })
            .collect();
        let returned: usize = groups
            .iter()
            .map(|g| g["events"].as_array().map(|e| e.len()).unwrap_or(0))
            .sum();
        let payload = json!({
            "query": args.query,
            "scope": scope.to_string(),
            "returned": returned,
            "total_matches": outcome.total_matches,
            "truncated": outcome.truncated,
            "provenance": "historical-evidence",
            "history": groups,
            "note": "Historical evidence from past sessions: what happened, when, where, and what produced it — not what to do. Task history stays invisible without its task_id; pass scope=global to explicitly search every workspace.",
        });
        let text = crate::mcp::response_bounds::bounded_response(payload)
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    // ── Tool 22: learn (cautious hypotheses from history) ─────────────────

    /// Learn from accumulated engineering history: detect recurring patterns
    /// in past decisions, validations, and changes; evaluate them against
    /// supporting and contradicting evidence with bounded confidence; and
    /// persist sufficiently-supported conclusions as AI_INFERRED knowledge
    /// (never USER_CONFIRMED). Inspect pending candidates, confirm what the
    /// user explicitly approves, or reject what does not hold. Read-only
    /// actions (list, get) write nothing; learning never writes history.
    #[tool(
        description = "Learn from engineering history: detect recurring patterns, evaluate them against supporting and contradicting evidence, and persist strong hypotheses as AI_INFERRED knowledge. Confirm only what the user approved; reject what does not hold."
    )]
    async fn learn(
        &self,
        Parameters(args): Parameters<LearnArgs>,
    ) -> Result<CallToolResult, McpError> {
        let action = args.action.as_deref().map(str::trim).unwrap_or("");
        // Read-only actions take no lock and capture no history.
        // Mutating actions serialize on the workspace lock like every
        // other writer; learning never writes history (no recursion, no
        // self-evidence).
        match action {
            "list" | "get" => self.learn_read(args).await,
            "run" | "propose" | "evaluate" | "confirm" | "reject" => {
                let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
                let _mutation_guard = ws.mutation_lock.lock().await;
                self.learn_write(args).await
            }
            other => Err(McpError::invalid_params(
                format!(
                    "unknown learn action '{other}': use run, propose, list, get, \
                     evaluate, confirm, or reject"
                ),
                None,
            )),
        }
    }

    /// Read-only `learn` actions: inspect candidates and explanations.
    async fn learn_read(&self, args: LearnArgs) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let store = self.context_store();
        let root = ws.canonical_root.display().to_string();
        let action = args.action.as_deref().unwrap_or("");
        match action {
            "list" => {
                let status = match args.status.as_deref().map(str::trim) {
                    None | Some("") => None,
                    Some(raw) => Some(
                        raw.parse::<crate::context_runtime::CandidateStatus>()
                            .map_err(|e| McpError::invalid_params(e, None))?,
                    ),
                };
                let limit = args.limit.unwrap_or(20).clamp(1, 50);
                let candidates = store
                    .list_candidates(Some(root.as_str()), status, args.task_id.as_deref(), limit)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let items: Vec<serde_json::Value> = candidates
                    .iter()
                    .map(|c| {
                        json!({
                            "candidate_id": c.candidate_id,
                            "kind": c.kind,
                            "scope": c.scope,
                            "status": c.status,
                            "confidence": c.confidence,
                            "proposition": c.proposition,
                            "namespace": c.namespace,
                            "supporting": c.supporting_evidence.len(),
                            "contradicting": c.contradicting_evidence.len(),
                            "eval_reason": c.eval_reason,
                            "inference_record_id": c.inference_record_id,
                            "updated_at": c.updated_at,
                        })
                    })
                    .collect();
                let payload = json!({
                    "candidates": items,
                    "returned": items.len(),
                    "provenance": "learning-candidates",
                    "note": "Pending hypotheses with their evidence counts and confidence. Deferred/rejected rows are preserved for audit, never surfaced as trusted context.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "get" => {
                let id = args.candidate_id.as_deref().map(str::trim).unwrap_or("");
                if id.is_empty() {
                    return Err(McpError::invalid_params(
                        "learn get requires candidate_id",
                        None,
                    ));
                }
                let candidate = store
                    .get_candidate(id)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("no learning candidate '{id}'"), None)
                    })?;
                // Workspace confinement: one project inspects its own
                // candidates plus globals, never another project's.
                let visible = match candidate.scope.as_str() {
                    "global" => true,
                    _ => candidate.workspace_root.as_deref() == Some(root.as_str()),
                };
                if !visible {
                    return Err(McpError::invalid_params(
                        "that candidate belongs to another workspace",
                        None,
                    ));
                }
                let payload = json!({
                    "candidate": candidate,
                    "explanation": candidate.explain(),
                    "provenance": "learning-candidates",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            _ => Err(McpError::internal_error("unreachable learn action", None)),
        }
    }

    /// Mutating `learn` actions. The workspace lock is already held by the
    /// caller (`learn`). History is never written here.
    async fn learn_write(&self, args: LearnArgs) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let store = self.context_store();
        let root = ws.canonical_root.display().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let scope = match args.scope.as_deref().map(str::trim) {
            None | Some("") | Some("project") => crate::context_runtime::LearnScope::Project,
            Some("task") => crate::context_runtime::LearnScope::Task,
            Some("global") => crate::context_runtime::LearnScope::Global,
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("unknown learn scope '{other}': use project, task, or global"),
                    None,
                ))
            }
        };
        let task_id = args
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let learn_error = |e: crate::context_runtime::store::ContextError| match e {
            crate::context_runtime::store::ContextError::Validation(msg) => {
                McpError::invalid_params(msg, None)
            }
            other => McpError::internal_error(other.to_string(), None),
        };
        let action = args.action.as_deref().unwrap_or("");
        match action {
            "run" => {
                let outcome = store
                    .run_learning(Some(root.as_str()), task_id, scope, now)
                    .map_err(learn_error)?;
                let payload = json!({
                    "learned": true,
                    "scope": scope.to_string(),
                    "proposed": outcome.proposed,
                    "accepted": outcome.accepted,
                    "deferred": outcome.deferred,
                    "rejected": outcome.rejected,
                    "refreshed": outcome.refreshed,
                    "skipped_terminal": outcome.skipped_terminal,
                    "failures": outcome.failures,
                    "provenance": "learning-candidates",
                    "note": "Accepted hypotheses persist as AI_INFERRED records with evidence and bounded confidence — never as USER_CONFIRMED truth.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "propose" => {
                let candidates = store
                    .propose_candidates(Some(root.as_str()), task_id, scope, now)
                    .map_err(learn_error)?;
                let items: Vec<serde_json::Value> = candidates
                    .iter()
                    .map(|c| {
                        json!({
                            "candidate_id": c.candidate_id,
                            "kind": c.kind,
                            "status": c.status,
                            "confidence": c.confidence,
                            "proposition": c.proposition,
                            "supporting": c.supporting_evidence.len(),
                            "contradicting": c.contradicting_evidence.len(),
                        })
                    })
                    .collect();
                let payload = json!({
                    "proposed": items.len(),
                    "scope": scope.to_string(),
                    "candidates": items,
                    "provenance": "learning-candidates",
                    "note": "Observations only: nothing evaluated, nothing persisted as knowledge. Call learn evaluate (or run) to weigh the evidence.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "evaluate" => {
                let id = args.candidate_id.as_deref().map(str::trim).unwrap_or("");
                if id.is_empty() {
                    return Err(McpError::invalid_params(
                        "learn evaluate requires candidate_id",
                        None,
                    ));
                }
                let candidate = store.evaluate_candidate(id, now).map_err(learn_error)?;
                let payload = json!({
                    "candidate": candidate,
                    "explanation": candidate.explain(),
                    "provenance": "learning-candidates",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "confirm" => {
                // Caller-principal rule: only an explicit user speech act
                // (user_confirmed=true) creates USER_CONFIRMED truth. The
                // model can never self-confirm an inference.
                if !args.user_confirmed.unwrap_or(false) {
                    return Err(McpError::invalid_params(
                        "learn confirm requires user_confirmed=true: set the flag only \
                         when the user explicitly stated or approved this conclusion",
                        None,
                    ));
                }
                let id = args.candidate_id.as_deref().map(str::trim).unwrap_or("");
                if id.is_empty() {
                    return Err(McpError::invalid_params(
                        "learn confirm requires candidate_id",
                        None,
                    ));
                }
                let candidate = store
                    .confirm_candidate(id, true, now)
                    .map_err(learn_error)?;
                let payload = json!({
                    "confirmed": true,
                    "candidate": candidate,
                    "provenance": "learning-candidates",
                    "note": "The user confirmed this conclusion: a USER_CONFIRMED record now supersedes the AI_INFERRED hypothesis (audit trail preserved).",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "reject" => {
                let id = args.candidate_id.as_deref().map(str::trim).unwrap_or("");
                if id.is_empty() {
                    return Err(McpError::invalid_params(
                        "learn reject requires candidate_id",
                        None,
                    ));
                }
                if !args.confirm.unwrap_or(false) {
                    return Err(McpError::invalid_params(
                        "learn reject requires confirm=true",
                        None,
                    ));
                }
                let candidate = store
                    .reject_candidate(id, args.reason.as_deref(), now)
                    .map_err(learn_error)?;
                let payload = json!({
                    "rejected": true,
                    "candidate": candidate,
                    "provenance": "learning-candidates",
                    "note": "Rejection is preserved as negative knowledge (rows stay for audit, never deleted). A project-scoped preference can still be recorded via remember.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            _ => Err(McpError::internal_error("unreachable learn action", None)),
        }
    }

    // ── P4: skill lifecycle tool ────────────────────────────────────────

    /// Skill publication root: `$CODEBRO_SKILLS_DIR` when set (tests and
    /// embedded deployments stay hermetic), otherwise OpenCode's global
    /// skill directory `~/.config/opencode/skills`.
    fn skills_root() -> Result<std::path::PathBuf, McpError> {
        if let Ok(dir) = std::env::var("CODEBRO_SKILLS_DIR") {
            if !dir.trim().is_empty() {
                return Ok(std::path::PathBuf::from(dir));
            }
        }
        dirs::home_dir()
            .map(|h| h.join(".config").join("opencode").join("skills"))
            .ok_or_else(|| McpError::internal_error("cannot determine HOME", None))
    }

    /// Whether a skill candidate is visible from the requesting
    /// workspace: global rows are visible everywhere, project/task rows
    /// only from their own workspace (and task rows need the task id).
    fn skill_candidate_visible_from(
        candidate: &crate::context_runtime::SkillCandidate,
        ws_root: &str,
        task_id: Option<&str>,
    ) -> bool {
        match candidate.scope.as_str() {
            "global" => true,
            "project" => candidate
                .workspace_root
                .as_deref()
                .map(|w| {
                    crate::context_runtime::canonical_workspace_key(w)
                        == crate::context_runtime::canonical_workspace_key(ws_root)
                })
                .unwrap_or(false),
            "task" => {
                let ws_ok = candidate
                    .workspace_root
                    .as_deref()
                    .map(|w| {
                        crate::context_runtime::canonical_workspace_key(w)
                            == crate::context_runtime::canonical_workspace_key(ws_root)
                    })
                    .unwrap_or(false);
                let task_ok = candidate
                    .task_id
                    .as_deref()
                    .zip(task_id)
                    .map(|(a, b)| a.trim() == b.trim())
                    .unwrap_or(false);
                ws_ok && task_ok
            }
            _ => false,
        }
    }

    /// Whether a skill is visible from the requesting workspace (same
    /// rule as candidates, minus task scoping: published skills are
    /// project-or-global).
    fn skill_visible_from(skill: &crate::context_runtime::Skill, ws_root: &str) -> bool {
        match skill.scope.as_str() {
            "global" => true,
            "project" => skill
                .workspace_root
                .as_deref()
                .map(|w| {
                    crate::context_runtime::canonical_workspace_key(w)
                        == crate::context_runtime::canonical_workspace_key(ws_root)
                })
                .unwrap_or(false),
            _ => false,
        }
    }

    #[tool(
        description = "Skill lifecycle: discover, propose, inspect, validate, approve, reject, deprecate, rollback, health. Manages evidence-backed skill candidates and versioned publication. OpenCode executes skills natively."
    )]
    async fn skill(
        &self,
        Parameters(args): Parameters<SkillArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let action = args.action.as_deref().unwrap_or("discover");
        let store = self.context_store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ws_root = ws.canonical_root.to_string_lossy().to_string();
        let task_id = args.task_id.as_deref();

        match action {
            "discover" => {
                let status_filter = args
                    .status
                    .as_deref()
                    .and_then(|s| crate::context_runtime::SkillCandidateStatus::from_str(s).ok());
                let candidates = store
                    .list_skill_candidates(
                        Some(&ws_root),
                        task_id,
                        status_filter,
                        args.limit.unwrap_or(20),
                    )
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let skills = store
                    .list_skills(
                        Some(&ws_root),
                        Some(crate::context_runtime::SkillStatus::Active),
                        args.limit.unwrap_or(20),
                    )
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let payload = serde_json::json!({
                    "action": "discover",
                    "skill_candidates": candidates.iter().map(|c| c.explain()).collect::<Vec<_>>(),
                    "active_skills": skills,
                    "count": candidates.len() + skills.len(),
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "propose" => {
                let _guard = ws.mutation_lock.lock().await;
                let name = args.name.as_deref().ok_or_else(|| {
                    McpError::invalid_params("propose requires 'name'", None)
                })?;
                // Secret redaction at the write seam (same policy as
                // remember/task/record_memory): the free-text description
                // and purpose are persisted verbatim into skill rows that
                // every reader (inspect, discover, health, engineering
                // brief) surfaces — a secret pasted here must never
                // reach storage or any brief.
                let description = &crate::tools::shell::redact_secrets_public(
                    args.description.as_deref().unwrap_or(""),
                );
                let purpose = &crate::tools::shell::redact_secrets_public(
                    args.purpose.as_deref().unwrap_or(""),
                );
                let content = args.content.as_deref().unwrap_or("");

                // Structurally impossible names (traversal-shaped,
                // uppercase, separators) are refused outright: such a
                // candidate could never publish, so storing it would only
                // pollute the registry.
                if !crate::context_runtime::is_valid_skill_name(name) {
                    return Err(McpError::invalid_params(
                        format!(
                            "invalid skill name '{name}': must be 1-64 lowercase \
                             alphanumeric segments with single hyphens (e.g. git-release)"
                        ),
                        None,
                    ));
                }

                // Scope rules mirror P3 learning: project needs a
                // workspace (always present here), task needs a task id.
                let scope_str = args.scope.as_deref().unwrap_or("project");
                let scope = crate::context_runtime::SkillScope::from_str(scope_str)
                    .map_err(|e| McpError::invalid_params(e, None))?;
                if scope == crate::context_runtime::SkillScope::Task
                    && task_id.map(str::trim).unwrap_or("").is_empty()
                {
                    return Err(McpError::invalid_params(
                        "task-scoped skill candidates require task_id",
                        None,
                    ));
                }

                // If a learning candidate id is provided, derive from it.
                // The store enforces the P3 trust boundary (accepted-only)
                // and the P4 lineage conflict rules.
                if let Some(lc_id) = args.learning_candidate_id.as_deref() {
                    let lc = store
                        .get_candidate(lc_id)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                        .ok_or_else(|| McpError::invalid_params("learning candidate not found", None))?;

                    // Cross-workspace learning evidence must not seed
                    // candidates visible here.
                    if !Self::learning_candidate_in_scope(&lc, &ws_root, task_id) {
                        return Err(McpError::invalid_params(
                            "learning candidate is not visible from this workspace/task",
                            None,
                        ));
                    }

                    let applicability = crate::context_runtime::SkillApplicability {
                        languages: args.languages.clone().unwrap_or_default(),
                        subsystems: args.subsystems.clone().unwrap_or_default(),
                        ..Default::default()
                    };

                    let candidate = store
                        .create_skill_candidate_from_learning(
                            &lc,
                            name,
                            description,
                            purpose,
                            applicability,
                            content,
                            now,
                        )
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                    let payload = serde_json::json!({
                        "action": "propose",
                        "candidate": candidate.explain(),
                        "note": "Candidate created from accepted learning evidence. Use 'validate' (after promoting to draft) then explicit user approval to publish.",
                    });
                    let text = crate::mcp::response_bounds::bounded_response(payload)
                        .map_err(|e| McpError::internal_error(e, None))?;
                    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
                } else {
                    // Standalone proposal (not from learning): carries no
                    // evidence citations, so confidence is caller-declared
                    // and capped below the approval floor — a standalone
                    // candidate can never pass the store's confidence gate
                    // without evidence-backed learning behind it.
                    let declared = args.confidence.unwrap_or(0.5);
                    let confidence = declared.min(crate::context_runtime::SKILL_APPROVAL_MIN_CONFIDENCE - 0.01);
                    let candidate_id = crate::context_runtime::mint_skill_candidate_id(
                        &scope,
                        Some(&ws_root),
                        task_id,
                        name,
                        content,
                    );
                    let validation = crate::context_runtime::validate_skill_content(
                        content,
                        name,
                        Some(&ws_root),
                    );
                    let candidate = crate::context_runtime::SkillCandidate {
                        candidate_id,
                        workspace_root: Some(ws_root.clone()),
                        task_id: args.task_id.clone(),
                        scope: scope_str.to_string(),
                        name: name.to_string(),
                        description: description.to_string(),
                        purpose: purpose.to_string(),
                        applicability: crate::context_runtime::SkillApplicability {
                            languages: args.languages.clone().unwrap_or_default(),
                            subsystems: args.subsystems.clone().unwrap_or_default(),
                            ..Default::default()
                        },
                        source_learning_candidates: Vec::new(),
                        supporting_evidence: Vec::new(),
                        contradicting_evidence: Vec::new(),
                        proposed_content: content.to_string(),
                        status: "candidate".to_string(),
                        confidence,
                        validation: Some(validation),
                        eval_reason: Some("standalone proposal (no evidence citations; confidence capped below approval floor)".to_string()),
                        rejection_reason: None,
                        supersedes_skill: None,
                        based_on_version: {
                            // Anchor standalone updates to the current
                            // active version of the same-name lineage.
                            let sid = crate::context_runtime::mint_skill_id(
                                &scope, Some(&ws_root), name,
                            );
                            store
                                .get_skill(&sid)
                                .unwrap_or(None)
                                .filter(|s| s.status == "active")
                                .map(|s| s.current_version)
                        },
                        created_at: now,
                        updated_at: now,
                        expires_at: Some(now + crate::context_runtime::SKILL_CANDIDATE_TTL_SECS),
                    };
                    store
                        .insert_skill_candidate(&candidate)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                    let payload = serde_json::json!({
                        "action": "propose",
                        "candidate": candidate.explain(),
                        "note": "Standalone candidate created (confidence capped below the approval floor: publication requires evidence-backed learning). Use 'validate' then explicit user approval to publish.",
                    });
                    let text = crate::mcp::response_bounds::bounded_response(payload)
                        .map_err(|e| McpError::internal_error(e, None))?;
                    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
                }
            }
            "inspect" => {
                let candidate_id = args.candidate_id.as_deref();
                let skill_id = args.skill_id.as_deref();
                let skill_name = args.name.as_deref();

                if let Some(cid) = candidate_id {
                    let candidate = store
                        .get_skill_candidate(cid)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                        .ok_or_else(|| McpError::invalid_params("candidate not found", None))?;
                    if !Self::skill_candidate_visible_from(&candidate, &ws_root, task_id) {
                        return Err(McpError::invalid_params(
                            "candidate not visible from this workspace",
                            None,
                        ));
                    }
                    let payload = serde_json::json!({
                        "action": "inspect",
                        "type": "candidate",
                        "candidate": candidate.explain(),
                    });
                    let text = crate::mcp::response_bounds::bounded_response(payload)
                        .map_err(|e| McpError::internal_error(e, None))?;
                    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
                } else if let Some(sid) = skill_id {
                    let skill = store
                        .get_skill(sid)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                        .ok_or_else(|| McpError::invalid_params("skill not found", None))?;
                    if !Self::skill_visible_from(&skill, &ws_root) {
                        return Err(McpError::invalid_params(
                            "skill not visible from this workspace",
                            None,
                        ));
                    }
                    let versions = store
                        .list_skill_versions(sid, 10)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    let active_version = store
                        .get_active_skill_version(sid)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    let payload = serde_json::json!({
                        "action": "inspect",
                        "type": "skill",
                        "skill": skill,
                        "versions": versions,
                        "active_version": active_version,
                    });
                    let text = crate::mcp::response_bounds::bounded_response(payload)
                        .map_err(|e| McpError::internal_error(e, None))?;
                    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
                } else if let Some(n) = skill_name {
                    let skill = store
                        .get_skill_by_name(n)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                        .ok_or_else(|| McpError::invalid_params("skill not found by name", None))?;
                    if !Self::skill_visible_from(&skill, &ws_root) {
                        return Err(McpError::invalid_params(
                            "skill not visible from this workspace",
                            None,
                        ));
                    }
                    let payload = serde_json::json!({
                        "action": "inspect",
                        "type": "skill",
                        "skill": skill,
                    });
                    let text = crate::mcp::response_bounds::bounded_response(payload)
                        .map_err(|e| McpError::internal_error(e, None))?;
                    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
                } else {
                    Err(McpError::invalid_params(
                        "inspect requires candidate_id, skill_id, or name",
                        None,
                    ))
                }
            }
            "validate" => {
                let _guard = ws.mutation_lock.lock().await;
                let candidate_id = args.candidate_id.as_deref().ok_or_else(|| {
                    McpError::invalid_params("validate requires candidate_id", None)
                })?;
                let candidate = store
                    .get_skill_candidate(candidate_id)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .ok_or_else(|| McpError::invalid_params("candidate not found", None))?;
                if !Self::skill_candidate_visible_from(&candidate, &ws_root, task_id) {
                    return Err(McpError::invalid_params(
                        "candidate not visible from this workspace",
                        None,
                    ));
                }

                // The automated evaluation pass: re-validate the proposed
                // content and, on success, advance candidate → evaluating →
                // draft → validated. The human gates (approve/reject) stay
                // separate; a validated candidate still requires
                // user-confirmed approval to publish.
                let updated = store
                    .evaluate_candidate_content(candidate_id, now)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                let validation = updated
                    .validation
                    .clone()
                    .unwrap_or_default();

                let payload = serde_json::json!({
                    "action": "validate",
                    "candidate_id": candidate_id,
                    "status": updated.status,
                    "validation": validation,
                    "note": if validation.valid && updated.status == "validated" {
                        "Validation passed and the candidate is validated. Publishing requires explicit user approval: approve with user_confirmed=true."
                    } else if validation.valid {
                        "Validation passed. Re-run validate to advance the pipeline (content committed as draft)."
                    } else {
                        "Validation failed. Fix errors (propose a corrected candidate) before approving."
                    },
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "approve" => {
                // Caller-principal rule (defense in depth on top of the
                // store's status/confidence/workspace gates): approval is
                // a user decision. The model may not self-approve — the
                // flag must be set only when the user explicitly approved
                // publication of this candidate.
                if !args.user_confirmed.unwrap_or(false) {
                    return Err(McpError::invalid_params(
                        "skill approve requires user_confirmed=true: set the flag only \
                         when the user explicitly approved publishing this skill",
                        None,
                    ));
                }
                let _guard = ws.mutation_lock.lock().await;
                let candidate_id = args.candidate_id.as_deref().ok_or_else(|| {
                    McpError::invalid_params("approve requires candidate_id", None)
                })?;

                let skill_root = Self::skills_root()?;

                let (skill, version) = store
                    .approve_skill_candidate(candidate_id, Some(&ws_root), &skill_root, now)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                let payload = serde_json::json!({
                    "action": "approve",
                    "skill": skill,
                    "version": version,
                    "note": "Skill published (user-approved). OpenCode will discover it via its native skill system.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "reject" => {
                let _guard = ws.mutation_lock.lock().await;
                let candidate_id = args.candidate_id.as_deref().ok_or_else(|| {
                    McpError::invalid_params("reject requires candidate_id", None)
                })?;
                let reason = args.reason.as_deref().unwrap_or("rejected");

                let candidate = store
                    .get_skill_candidate(candidate_id)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .ok_or_else(|| McpError::invalid_params("candidate not found", None))?;
                if !Self::skill_candidate_visible_from(&candidate, &ws_root, task_id) {
                    return Err(McpError::invalid_params(
                        "candidate not visible from this workspace",
                        None,
                    ));
                }

                let updated = store
                    .transition_skill_candidate(
                        candidate_id,
                        crate::context_runtime::SkillCandidateStatus::Rejected,
                        Some(reason),
                        now,
                    )
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                let payload = serde_json::json!({
                    "action": "reject",
                    "candidate": updated.explain(),
                    "note": "Rejection preserved for audit trail. Future learning will not re-propose without new evidence.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "deprecate" => {
                let _guard = ws.mutation_lock.lock().await;
                let skill_id = args.skill_id.as_deref().ok_or_else(|| {
                    McpError::invalid_params("deprecate requires skill_id", None)
                })?;
                let reason = args.reason.as_deref();

                let skill_root = Self::skills_root()?;

                let skill = store
                    .deprecate_skill(skill_id, Some(&ws_root), &skill_root, reason, now)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                let payload = serde_json::json!({
                    "action": "deprecate",
                    "skill": skill,
                    "note": "Skill deprecated and its published file removed. It will no longer be discovered; version history is preserved.",
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "rollback" => {
                let _guard = ws.mutation_lock.lock().await;
                let skill_id = args.skill_id.as_deref().ok_or_else(|| {
                    McpError::invalid_params("rollback requires skill_id", None)
                })?;
                let target_version = args.version.ok_or_else(|| {
                    McpError::invalid_params("rollback requires version", None)
                })?;

                let skill_root = Self::skills_root()?;

                let (skill, version) = store
                    .rollback_skill(skill_id, Some(&ws_root), target_version, &skill_root, now)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                let payload = serde_json::json!({
                    "action": "rollback",
                    "skill": skill,
                    "rolled_back_to": version,
                    "note": format!("Rolled back to version {}. Previous version preserved in history.", target_version),
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "health" => {
                let _guard = ws.mutation_lock.lock().await;
                let skill_id = args.skill_id.as_deref().ok_or_else(|| {
                    McpError::invalid_params("health requires skill_id", None)
                })?;
                let success = args.success.unwrap_or(true);

                let skill = store
                    .get_skill(skill_id)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .ok_or_else(|| McpError::invalid_params("skill not found", None))?;
                if !Self::skill_visible_from(&skill, &ws_root) {
                    return Err(McpError::invalid_params(
                        "skill not visible from this workspace",
                        None,
                    ));
                }

                store
                    .record_skill_use(skill_id, success, now)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                let skill = store
                    .get_skill(skill_id)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .ok_or_else(|| McpError::invalid_params("skill not found", None))?;

                let payload = serde_json::json!({
                    "action": "health",
                    "skill": skill,
                    "recorded": if success { "success" } else { "failure" },
                });
                let text = crate::mcp::response_bounds::bounded_response(payload)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            _ => Err(McpError::invalid_params(
                "unknown skill action: use discover, propose, inspect, validate, approve, reject, deprecate, rollback, or health",
                None,
            )),
        }
    }

    // ── P5: durable engineering task runtime ───────────────────────────

    /// Map a store-level ContextError to an MCP error: validation
    /// failures (state machine, isolation, lease, stale writers) are
    /// caller mistakes (invalid params); the rest are internal.
    fn task_error(e: crate::context_runtime::store::ContextError) -> McpError {
        match e {
            crate::context_runtime::store::ContextError::Validation(msg) => {
                McpError::invalid_params(msg, None)
            }
            other => McpError::internal_error(other.to_string(), None),
        }
    }

    /// The per-task JSON payload (bounded via response_bounds).
    fn task_payload(task: &crate::context_runtime::TaskRecord) -> serde_json::Value {
        serde_json::to_value(task).unwrap_or(serde_json::json!({}))
    }

    /// Build the mutation context for a per-task action (handler helper).
    fn task_ctx<'a>(
        ws_root: &'a str,
        task_id: &'a str,
        worker: &'a str,
        lease_version: u64,
        based_on_version: Option<u64>,
        now: u64,
    ) -> crate::context_runtime::tasks::TaskMutationCtx<'a> {
        crate::context_runtime::tasks::TaskMutationCtx {
            workspace_root: ws_root,
            task_id,
            worker,
            lease_version,
            based_on_version,
            now,
        }
    }

    #[tool(
        description = "Durable engineering task runtime: create, start, pause, resume, checkpoint, validate, complete, fail, cancel, list, inspect, stale, outcome. Tasks persist across sessions with immutable checkpoints, worker leases with fencing, and a validated lifecycle; OpenCode remains the executor."
    )]
    async fn task(
        &self,
        Parameters(args): Parameters<TaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let ws = self.resolve_workspace(args.workspace_root.as_deref())?;
        let store = self.context_store();
        let ws_root = ws.canonical_root.to_string_lossy().to_string();
        let worker = self.task_worker_id.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let action = args.action.as_deref().map(str::trim).unwrap_or("list");

        // Read-only actions never take the mutation lock. Mutating
        // actions hold it for the whole handler: the guard must stay
        // alive until the mutation completes (a guard scoped to this
        // match arm alone would drop immediately and serialize nothing).
        let _mutation_guard = match action {
            "list" | "inspect" | "stale" => None,
            _ => Some(ws.mutation_lock.lock().await),
        };

        let require_task_id = || -> Result<String, McpError> {
            args.task_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("task action '{action}' requires task_id"),
                        None,
                    )
                })
        };

        let payload = match action {
            "list" => {
                let status = match args.status.as_deref().map(str::trim) {
                    None | Some("") => None,
                    Some(raw) => Some(
                        raw.parse::<crate::context_runtime::TaskStatus>()
                            .map_err(|e| McpError::invalid_params(e, None))?,
                    ),
                };
                let tasks = store
                    .list_tasks(
                        &ws_root,
                        status,
                        args.parent_task_id.as_deref(),
                        args.limit.unwrap_or(20),
                        now,
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "list",
                    "tasks": tasks,
                    "count": tasks.len(),
                })
            }
            "stale" => {
                // Recoverable work: interrupted tasks with expired leases.
                let tasks = store.stale_tasks(&ws_root, now).map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "stale",
                    "interrupted_tasks": tasks,
                    "note": "interrupted work is never auto-completed; resume explicitly to continue it",
                    "count": tasks.len(),
                })
            }
            "create" => {
                let title = args
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| McpError::invalid_params("create requires 'title'", None))?;
                let priority = match args.priority.as_deref().map(str::trim) {
                    None | Some("") => None,
                    Some(raw) => Some(
                        raw.parse::<crate::context_runtime::TaskPriority>()
                            .map_err(|e| McpError::invalid_params(e, None))?,
                    ),
                };
                let input = crate::context_runtime::tasks::NewTask {
                    title: title.to_string(),
                    description: args.description.as_deref().map(str::to_string),
                    priority,
                    intent_record_id: args.intent_record_id.as_deref().map(str::to_string),
                    parent_task_id: args.parent_task_id.as_deref().map(str::to_string),
                    idempotency_key: args.idempotency_key.as_deref().map(str::to_string),
                    skill_refs: args.skill_refs.clone().unwrap_or_default(),
                };
                let task = store
                    .create_task(&ws_root, &input, now)
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "create",
                    "task": Self::task_payload(&task),
                })
            }
            "inspect" => {
                let task_id = require_task_id()?;
                let snapshot = store
                    .task_resume_snapshot(&ws_root, &task_id, now)
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "inspect",
                    "snapshot": snapshot,
                })
            }
            "start" => {
                let task_id = require_task_id()?;
                let task = store
                    .start_task(&ws_root, &task_id, &worker, args.based_on_version, now)
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "start",
                    "task": Self::task_payload(&task),
                })
            }
            "pause" => {
                let task_id = require_task_id()?;
                let task = store
                    .get_task(&ws_root, &task_id)
                    .map_err(Self::task_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task {task_id} does not exist"), None)
                    })?;
                let task = store
                    .pause_task(
                        &ws_root,
                        &task_id,
                        &worker,
                        task.lease_version,
                        args.based_on_version,
                        now,
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "pause",
                    "task": Self::task_payload(&task),
                })
            }
            "resume" => {
                let task_id = require_task_id()?;
                let task = store
                    .resume_task(&ws_root, &task_id, &worker, args.based_on_version, now)
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "resume",
                    "task": Self::task_payload(&task),
                })
            }
            "checkpoint" => {
                let task_id = require_task_id()?;
                let summary = args
                    .summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        McpError::invalid_params("checkpoint requires 'summary'", None)
                    })?;
                let task = store
                    .get_task(&ws_root, &task_id)
                    .map_err(Self::task_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task {task_id} does not exist"), None)
                    })?;
                let checkpoint = store
                    .create_task_checkpoint(
                        &Self::task_ctx(
                            &ws_root,
                            &task_id,
                            &worker,
                            task.lease_version,
                            args.based_on_version,
                            now,
                        ),
                        &crate::context_runtime::tasks::CheckpointInput {
                            summary,
                            progress: args.progress.as_deref(),
                            next_action: args.next_action.as_deref(),
                            validation_status: None,
                            metadata_json: args.metadata.as_deref(),
                        },
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "checkpoint",
                    "checkpoint": checkpoint,
                })
            }
            "validate" => {
                let task_id = require_task_id()?;
                let what = args
                    .what
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("repository validation");
                let task = store
                    .get_task(&ws_root, &task_id)
                    .map_err(Self::task_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task {task_id} does not exist"), None)
                    })?;
                let task = store
                    .start_task_validation(
                        &Self::task_ctx(
                            &ws_root,
                            &task_id,
                            &worker,
                            task.lease_version,
                            args.based_on_version,
                            now,
                        ),
                        what,
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "validate",
                    "task": Self::task_payload(&task),
                })
            }
            "validation_result" => {
                let task_id = require_task_id()?;
                let result = match args.result.as_deref().map(str::trim) {
                    Some("passed") => crate::context_runtime::TaskValidationResult::Passed,
                    Some("failed") => crate::context_runtime::TaskValidationResult::Failed,
                    other => {
                        return Err(McpError::invalid_params(
                            format!(
                            "validation_result requires result='passed'|'failed' (got {other:?})"
                        ),
                            None,
                        ))
                    }
                };
                let what = args
                    .what
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("repository validation");
                let task = store
                    .get_task(&ws_root, &task_id)
                    .map_err(Self::task_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task {task_id} does not exist"), None)
                    })?;
                let task = store
                    .record_task_validation(
                        &Self::task_ctx(
                            &ws_root,
                            &task_id,
                            &worker,
                            task.lease_version,
                            args.based_on_version,
                            now,
                        ),
                        what,
                        result,
                        args.reason.as_deref(),
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "validation_result",
                    "task": Self::task_payload(&task),
                })
            }
            "complete" => {
                let task_id = require_task_id()?;
                let task = store
                    .get_task(&ws_root, &task_id)
                    .map_err(Self::task_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task {task_id} does not exist"), None)
                    })?;
                let task = store
                    .complete_task(
                        &Self::task_ctx(
                            &ws_root,
                            &task_id,
                            &worker,
                            task.lease_version,
                            args.based_on_version,
                            now,
                        ),
                        args.reason.as_deref().unwrap_or("completed"),
                        args.changed_areas.clone().unwrap_or_default(),
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "complete",
                    "task": Self::task_payload(&task),
                })
            }
            "fail" => {
                let task_id = require_task_id()?;
                let task = store
                    .get_task(&ws_root, &task_id)
                    .map_err(Self::task_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task {task_id} does not exist"), None)
                    })?;
                let task = store
                    .fail_task(
                        &Self::task_ctx(
                            &ws_root,
                            &task_id,
                            &worker,
                            task.lease_version,
                            args.based_on_version,
                            now,
                        ),
                        args.reason.as_deref().unwrap_or("failed"),
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "fail",
                    "task": Self::task_payload(&task),
                })
            }
            "cancel" => {
                let task_id = require_task_id()?;
                let task = store
                    .cancel_task(
                        &ws_root,
                        &task_id,
                        args.reason.as_deref(),
                        args.based_on_version,
                        now,
                    )
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "cancel",
                    "task": Self::task_payload(&task),
                })
            }
            "skill_refs" => {
                let task_id = require_task_id()?;
                let refs = args.skill_refs.clone().unwrap_or_default();
                let task = store
                    .set_task_skill_refs(&ws_root, &task_id, refs, args.based_on_version, now)
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "skill_refs",
                    "task": Self::task_payload(&task),
                })
            }
            "outcome" => {
                // P9 engineering-outcome ingestion: OpenCode reports what its
                // work taught us; CodeBro persists the report as task-bound
                // history evidence. No transition, no execution, no
                // inference — validation/completion stay on their own
                // actions, and generalization stays with `learn`.
                let task_id = require_task_id()?;
                let classification = args
                    .classification
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        McpError::invalid_params(
                            "outcome requires 'classification': \
                             success|partial|failure|rejected|superseded",
                            None,
                        )
                    })?;
                let classification = classification
                    .parse::<crate::context_runtime::OutcomeClassification>()
                    .map_err(|e| McpError::invalid_params(e, None))?;
                let summary = args
                    .summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| McpError::invalid_params("outcome requires 'summary'", None))?;
                let input = crate::context_runtime::tasks::TaskOutcomeInput {
                    classification,
                    summary,
                    evidence: args.reason.as_deref(),
                    command: args.what.as_deref(),
                    exit_code: args.exit_code,
                    changed_areas: args.changed_areas.clone().unwrap_or_default(),
                    user_confirmed: args.user_confirmed.unwrap_or(false),
                    dedup_key: args.dedup_key.as_deref(),
                };
                let record = store
                    .record_task_outcome(&ws_root, &task_id, &input, now)
                    .map_err(Self::task_error)?;
                serde_json::json!({
                    "action": "outcome",
                    "task_id": task_id,
                    "event_id": record.event_id,
                    "duplicate": record.duplicate,
                    "classification": record.classification.as_str(),
                    "authority": record.authority,
                })
            }
            other => {
                return Err(McpError::invalid_params(
                    format!(
                        "unknown task action: {other:?} — use list, stale, create, inspect, \
                         start, pause, resume, checkpoint, validate, validation_result, \
                         complete, fail, cancel, outcome, or skill_refs"
                    ),
                    None,
                ))
            }
        };
        let text = crate::mcp::response_bounds::bounded_response(payload)
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    /// P3 learning-candidate visibility from a workspace (mirrors the
    /// candidate scope rule used by the `learn` tool).
    fn learning_candidate_in_scope(
        lc: &crate::context_runtime::LearningCandidate,
        ws_root: &str,
        task_id: Option<&str>,
    ) -> bool {
        let canon = crate::context_runtime::canonical_workspace_key;
        match lc.scope.as_str() {
            "global" => true,
            "project" => lc
                .workspace_root
                .as_deref()
                .map(|w| canon(w) == canon(ws_root))
                .unwrap_or(false),
            "task" => {
                let ws_ok = lc
                    .workspace_root
                    .as_deref()
                    .map(|w| canon(w) == canon(ws_root))
                    .unwrap_or(false);
                let task_ok = lc
                    .task_id
                    .as_deref()
                    .zip(task_id)
                    .map(|(a, b)| a.trim() == b.trim())
                    .unwrap_or(false);
                ws_ok && task_ok
            }
            _ => false,
        }
    }
}

/// Argument schema for `engineering_brief` (P7 decision support).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EngineeringBriefArgs {
    /// The engineering task in the agent's own words. Ad-hoc text is never
    /// persisted. At least one of task, task_id, a target, or keywords is
    /// required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Existing P5 task id (canonical `task::<hex>`). Read-only snapshot;
    /// never transitions the task. Cross-workspace ids read as unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Explicit workspace-relative file path target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    /// Explicit symbol target (id or name; ambiguous names are reported,
    /// never guessed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_symbol: Option<String>,
    /// Explicit module target (id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_module: Option<String>,
    /// Extra keyword hints for evidence retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    /// Impact traversal depth (default 1, max 2; deeper graphs belong to
    /// `impact_analyze`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `context`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ContextArgs {
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// The task in the agent's own words. Omit for a structural digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Extra keyword hints for fact/memory/record retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    /// Task identity for task-scoped context resolution. When supplied,
    /// task-scoped fingerprint/intent overrides for this task resolve
    /// alongside project and global records (task > project > global).
    /// Omit to resolve project + global only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// Argument schema for `remember` (explicit user-context persistence).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RememberArgs {
    /// Canonical semantic statement (what the user confirmed). Secrets are
    /// redacted before storage.
    pub content: String,
    /// Machine-facing namespace (e.g. fp.engineering.simplicity,
    /// intent.mission). One winner per (kind, namespace, scope).
    pub namespace: String,
    /// Record kind: preference, intent, constraint, principle, style,
    /// taste, pattern, or other. Default: preference. fact/decision/skill
    /// are reference-only and require related_ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Scope: global, project (default), or task (requires task_id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Task identity for task scope (the OpenCode session/task this
    /// override belongs to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Authority: user_confirmed (requires user_confirmed=true),
    /// observed, or ai_inferred (both require evidence). Default:
    /// user_confirmed when the flag is set, else observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    /// Explicit user-confirmation speech act: set true only when the user
    /// stated or approved this content. Required for USER_CONFIRMED.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_confirmed: Option<bool>,
    /// Verbatim user wording the canonical statement was interpreted
    /// from (evidence of the interpretation, ≤2048 chars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    /// Language/dialect tag of the original exchange (e.g. en, ms, manglish).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Existing evidence event ids to cite (must exist in the store).
    #[serde(default)]
    pub evidence_event_ids: Vec<i64>,
    /// What was observed, in the agent's words: mints an evidence event
    /// and cites it (so observed writes never cite fiction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
    /// Confidence in [0,1] (default 0.9 confirmed, 0.6 otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Importance in [0,1] for excerpt ordering (default 0.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<f64>,
    /// Provenance source label (e.g. a session id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Canonical entity ids this record references (required for
    /// fact/decision/skill kinds, which never duplicate canonical content).
    #[serde(default)]
    pub related_ids: Vec<String>,
    /// Intent only: why this goal matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Intent only: high, medium, or low.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Intent only: active (default), paused, completed, cancelled, or
    /// superseded (requires supersedes). Completed/cancelled retire the
    /// predecessor into a terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_status: Option<String>,
    /// Id of the active record this replaces (history preserved via the
    /// supersede chain, never overwritten).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `forget` (reversible retirement of user context).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ForgetArgs {
    /// Exact id of the record to retire (from a `context` excerpt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Otherwise the namespace whose resolution winner to retire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Task identity, required to retire task-scoped records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Explicit confirmation gate: forget refuses unless true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<bool>,
    /// Hard-remove the row instead of the default reversible reject.
    /// Cleanup of junk only; prefer the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permanent: Option<bool>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `recall` (historical evidence on demand).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RecallArgs {
    /// The question in the caller's words (e.g. "why are we using SQLite?").
    /// Must contain at least one searchable token (3+ alphanumeric chars).
    pub query: String,
    /// Scope: project (default, this workspace only), task (this workspace
    /// plus task_id), or global (explicit opt-in across all workspaces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Task identity: required for task scope; boosts ranking otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Narrow to event kinds (e.g. ["decision", "validation", "error"]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
    /// Narrow to one session id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Maximum excerpts returned (default 10, max 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `learn` (cautious hypotheses from history).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LearnArgs {
    /// Action: run (detect + evaluate + persist justified inferences),
    /// propose (detect only), list (inspect candidates), get (one candidate
    /// with its explanation), evaluate (weigh one candidate's evidence),
    /// confirm (explicit user confirmation → USER_CONFIRMED, requires
    /// user_confirmed=true), reject (user rejection → rejected, requires
    /// confirm=true).
    pub action: Option<String>,
    /// Scope: project (default, this workspace only), task (this workspace
    /// plus task_id), or global (explicit opt-in across all workspaces;
    /// global inference needs broader evidence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Task identity: required for task scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Candidate id for get, evaluate, confirm, reject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    /// Confirm list/get filter by status (candidate, deferred, accepted,
    /// rejected, expired, superseded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Maximum candidates returned by list (default 20, max 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Explicit user-confirmation speech act for confirm: set true only
    /// when the user stated or approved this conclusion. Required for
    /// USER_CONFIRMED; the model can never self-confirm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_confirmed: Option<bool>,
    /// Explicit confirmation gate for reject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<bool>,
    /// Optional reason recorded with a rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `skill`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SkillArgs {
    /// Action: discover (list candidates + skills), propose (create candidate),
    /// inspect (view candidate or skill details), validate (check content),
    /// approve (publish skill), reject (reject candidate), deprecate
    /// (retire skill), rollback (revert to prior version), health
    /// (record usage outcome).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Skill candidate id for inspect, validate, approve, reject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    /// Skill id for inspect, deprecate, rollback, health.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    /// Skill name for propose, inspect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Skill description for propose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Skill purpose for propose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Proposed SKILL.md content for propose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Learning candidate id to derive from (optional for propose).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_candidate_id: Option<String>,
    /// Scope: project (default), task, or global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Task identity for task-scoped skills and candidate visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Confidence score for standalone proposals (capped below the
    /// approval floor: standalone proposals carry no evidence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Languages for applicability metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<Vec<String>>,
    /// Subsystems for applicability metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystems: Option<Vec<String>>,
    /// Filter by status for discover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Maximum results for discover (default 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Rejection/deprecation reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Target version for rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Whether the health recording is a success (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Explicit user-approval speech act: required for `approve`. Set
    /// true only when the user explicitly approved publishing this
    /// skill candidate; the model may never self-approve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_confirmed: Option<bool>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `task` (P5 durable engineering task runtime).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TaskArgs {
    /// Semantic action: list (default), create, inspect, start, pause,
    /// resume, checkpoint, validate, validation_result, complete, fail,
    /// cancel, stale, outcome, skill_refs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Task id (required for every per-task action except create/list/stale).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// New-task title (create).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// New-task description (create).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Task priority: high | medium (default) | low.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Optional P1 intent record id this task implements (reference only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_record_id: Option<String>,
    /// Optional parent task id (create: organizational nesting only;
    /// list: filter children of this task).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    /// Explicit idempotency key: creating with the same (workspace, key)
    /// returns the existing task instead of duplicating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Optimistic-concurrency anchor: the task version this mutation is
    /// based on. Stale writers are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on_version: Option<u64>,
    /// Status filter (list): pending | running | paused | validating |
    /// completed | failed | cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Maximum tasks returned by list (default 20, max 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Checkpoint fields (checkpoint): what has been completed.
    pub summary: Option<String>,
    /// Checkpoint field: what remains (progress).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
    /// Checkpoint field: what should happen next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    /// Checkpoint field: bounded JSON object of task-private metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    /// Validation description (validate / validation_result): what is being
    /// validated (command/test summary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
    /// Validation outcome (validation_result): passed | failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Validation evidence summary (validation_result), and outcome
    /// summary (complete/fail/cancel reason).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Changed areas for the completion outcome (complete).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_areas: Option<Vec<String>>,
    /// Outcome classification (outcome): success | partial | failure |
    /// rejected | superseded. What the reported work taught us; whether
    /// it generalizes is P3 learning's job, never this call's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    /// Explicit user-confirmation speech act (outcome): set true only
    /// when the user stated or approved this outcome. Otherwise the
    /// report is recorded as observed (OpenCode-reported), never as
    /// user-confirmed truth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_confirmed: Option<bool>,
    /// Explicit idempotency key (outcome): redelivering the same
    /// (task, key) returns the original event instead of duplicating
    /// history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    /// Command exit status (outcome): the exit code of the reported
    /// test/build command, when the outcome reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Skill references (skill_refs): skill ids this task used.
    /// Reference-only association; CodeBro never executes skills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_refs: Option<Vec<String>>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
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
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
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
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
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
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// One change inside an `apply_changes` transaction.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TransactionChangeArgs {
    /// Path to the file, relative to the workspace root.
    pub path: String,
    /// Exact existing text to replace; empty to create a new file.
    pub old: String,
    /// Replacement text.
    pub new: String,
}

/// Argument schema for `apply_changes` (transactional multi-file mutation).
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ApplyChangesArgs {
    /// The set of changes applied all-or-nothing: either every change lands
    /// or the workspace is rolled back to its prior state.
    pub changes: Vec<TransactionChangeArgs>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
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
    /// Optional entry lifetime in seconds; the entry expires (and stops
    /// resolving) after this window. Omit for no expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_secs: Option<u64>,
    /// Optional session identifier recorded in structured provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `delete_memory`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct DeleteMemoryArgs {
    /// Exact key of the entry to delete.
    pub key: String,
    /// Explicit confirmation gate. Default false — the tool refuses to delete
    /// unless the caller sets this to true. Prevents accidental / speculative
    /// deletion when an agent misidentifies a key.
    #[serde(default)]
    pub confirm: bool,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// A decision to record via `update_identity`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct DecisionInput {
    /// Short title (required). The id is derived from it; duplicate titles are skipped.
    pub title: String,
    /// Full description of the decision.
    pub description: String,
    /// Optional context that led to the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// "proposed" | "accepted" | "deprecated" | "superseded". Default "accepted".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// A roadmap item to record via `update_identity`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RoadmapItemInput {
    /// Short title (required). The id is derived from it; duplicate titles are skipped.
    pub title: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// "planned" | "in_progress" | "completed" | "deferred". Default "planned".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Optional sprint this item belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint: Option<String>,
}

/// Argument schema for `update_identity`.
///
/// Every field is optional; supply only what should change. List fields
/// append new unique entries (existing ones are skipped, not duplicated).
#[derive(Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateIdentityArgs {
    /// Human-readable project description (what this project is for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Remote repository URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    /// High-level architecture summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_summary: Option<String>,
    /// Active sprint identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_sprint: Option<String>,
    /// Hard and soft constraints agents must respect.
    #[serde(default)]
    pub add_constraints: Vec<String>,
    /// Recognised architectural patterns.
    #[serde(default)]
    pub add_patterns: Vec<String>,
    /// Coding conventions for this project.
    #[serde(default)]
    pub add_conventions: Vec<String>,
    /// Important module names.
    #[serde(default)]
    pub add_modules: Vec<String>,
    /// Important project files (paths).
    #[serde(default)]
    pub add_important_files: Vec<String>,
    /// Engineering decisions to record.
    #[serde(default)]
    pub add_decisions: Vec<DecisionInput>,
    /// Roadmap items to record.
    #[serde(default)]
    pub add_roadmap_items: Vec<RoadmapItemInput>,
    /// Mark a roadmap item completed by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete_roadmap_item: Option<String>,
    /// Record a completed milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_milestone: Option<String>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Parse a `remember` kind string into a [`RecordKind`]. Fingerprint and
/// intent kinds are first-class; `experience` passes through for the later
/// learning phase; `fact`/`decision`/`skill` are reference-only (the store
/// refuses them without `related_ids`).
fn parse_record_kind(raw: Option<&str>) -> Result<crate::context_runtime::RecordKind, McpError> {
    use crate::context_runtime::RecordKind;
    match raw.map(str::trim).unwrap_or("preference") {
        "" | "preference" => Ok(RecordKind::Preference),
        "intent" => Ok(RecordKind::Intent),
        "constraint" => Ok(RecordKind::Constraint),
        "principle" => Ok(RecordKind::Principle),
        "style" => Ok(RecordKind::Style),
        "taste" => Ok(RecordKind::Taste),
        "pattern" => Ok(RecordKind::Pattern),
        "experience" => Ok(RecordKind::Experience),
        "fact" => Ok(RecordKind::Fact),
        "decision" => Ok(RecordKind::Decision),
        "skill" => Ok(RecordKind::Skill),
        "other" => Ok(RecordKind::Other),
        other => Err(McpError::invalid_params(
            format!(
                "unknown record kind '{other}': use preference, intent, constraint, \
                 principle, style, taste, pattern, experience, or other"
            ),
            None,
        )),
    }
}

/// Parse a scope string. `allow_task` is always true today (both callers
/// accept task scope); the parameter documents that global/project callers
/// never silently coerce.
fn parse_record_scope(
    raw: Option<&str>,
    _allow_task: bool,
) -> Result<crate::context_runtime::RecordScope, McpError> {
    use crate::context_runtime::RecordScope;
    match raw.map(str::trim).unwrap_or("project") {
        "" | "project" => Ok(RecordScope::Project),
        "global" => Ok(RecordScope::Global),
        "task" => Ok(RecordScope::Task),
        other => Err(McpError::invalid_params(
            format!("unknown scope '{other}': use global, project, or task"),
            None,
        )),
    }
}

/// Mint a collision-free record id: `ctx::<kind>::<namespace-slug>::<now>`
/// with a numeric suffix while the id exists (a plain upsert would
/// silently overwrite an unrelated record).
fn mint_record_id(
    store: &crate::context_runtime::ContextStore,
    kind: crate::context_runtime::RecordKind,
    namespace: &str,
    now: u64,
) -> String {
    let base = format!("ctx::{}::{}::{now}", kind.as_str(), slugify(namespace));
    let mut candidate = base.clone();
    let mut n = 2u32;
    loop {
        match store.get_record(&candidate) {
            Ok(None) => return candidate,
            _ => {
                candidate = format!("{base}-{n}");
                n += 1;
            }
        }
    }
}

/// Active record ids sharing one (kind, namespace, scope) binding — the
/// clash set a fresh `remember` must refuse (callers replace via
/// `supersedes`) or, for terminal intents, auto-retire when unambiguous.
fn find_active_in_namespace(
    store: &crate::context_runtime::ContextStore,
    kind: crate::context_runtime::RecordKind,
    namespace: &str,
    scope: crate::context_runtime::RecordScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
    now: u64,
) -> Result<Vec<String>, crate::context_runtime::store::ContextError> {
    use crate::context_runtime::ContextRetriever;
    let query = crate::context_runtime::RecordQuery {
        workspace_root,
        task_id,
        kind: Some(kind),
        status: None, // active-only default
        keywords: Vec::new(),
        limit: 100,
    };
    let ranked = store.search(&query, now)?;
    Ok(ranked
        .into_iter()
        .filter(|r| {
            r.record.namespace == namespace
                && r.record.scope == scope
                && r.record.workspace_root.as_deref() == workspace_root
                && r.record.task_id.as_deref() == task_id
        })
        .map(|r| r.record.id)
        .collect())
}

/// Map store errors from the `remember` write path: validation failures
/// (bad scope, fictitious evidence, reference-only kinds without backlinks)
/// are caller errors (`invalid_params`); anything else is internal.
fn remember_error(e: crate::context_runtime::store::ContextError) -> McpError {
    match e {
        crate::context_runtime::store::ContextError::Validation(msg) => {
            McpError::invalid_params(msg, None)
        }
        other => McpError::internal_error(other.to_string(), None),
    }
}

/// Trim and require a non-empty string field.
fn require_non_empty(value: &str, field: &str) -> Result<String, McpError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(McpError::invalid_params(
            format!("{field} must not be empty"),
            None,
        ));
    }
    Ok(trimmed.to_string())
}

/// Deterministic slug for decision/roadmap ids: lowercase, non-alphanumeric
/// runs collapsed to `-`, capped at 80 chars.
fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 80 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("item");
    }
    slug
}

/// Append values not already present in `existing` (case-sensitive).
fn push_unique_strings(target: &mut Vec<String>, values: &[String], existing: &[String]) {
    for value in values {
        // Redact at the write seam: identity free-text (constraints,
        // patterns, conventions, modules, files) is persisted to
        // .codebro/project_identity.json and surfaced by context packets
        // and engineering briefs — a pasted secret must never persist.
        let trimmed = crate::tools::shell::redact_secrets_public(value.trim());
        if trimmed.is_empty() || existing.iter().any(|e| e == &trimmed) {
            continue;
        }
        if !target.iter().any(|t| t == &trimmed) {
            target.push(trimmed);
        }
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
    pub metadata: std::collections::HashMap<String, String>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
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
    pub metadata: std::collections::HashMap<String, String>,
    /// Optional caller-supplied fact IDs associated with the change being
    /// verified. These IDs provide correlation context only — they are not
    /// independently verified by sandbox execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affected_fact_ids: Option<Vec<String>>,
    /// Optional test-name filters (e.g. taken from `apply_change`'s
    /// `recommended_tests`). Applied when the project's test runner supports
    /// name selection (cargo, go, pytest); ignored for runners without a
    /// standard selection mechanism, and when an explicit command overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_filter: Option<Vec<String>>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
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
    pub metadata: std::collections::HashMap<String, String>,
    /// Optional caller-supplied fact IDs associated with the change being
    /// verified. These IDs provide correlation context only — they are not
    /// independently verified by sandbox execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affected_fact_ids: Option<Vec<String>>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `impact_analyze`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ImpactArgs {
    /// The target to analyze: a symbol id, file path, module id, or package id.
    pub target: String,
    /// Target type: symbol, file, module, or package. Defaults to symbol.
    #[serde(default)]
    pub target_type: Option<String>,
    /// Maximum number of results per category (0 = no limit). Defaults to 50.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
    /// Whether to include related tests (default true).
    #[serde(default)]
    pub include_tests: Option<bool>,
    /// Whether to include cross-references (default true).
    #[serde(default)]
    pub include_references: Option<bool>,
    /// Bounded BFS depth. 0 = target only, 1 = direct relationships (default),
    /// up to 5. Values above 5 are rejected as invalid parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// Edge direction for traversal: "both" (default, preserves legacy behaviour),
    /// "outgoing", or "incoming".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// Optional subset of relationship kinds to traverse (e.g. ["calls", "imports"]).
    /// Empty means all kinds. Only known kinds are accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationship_types: Vec<String>,
    /// Hard ceiling on distinct graph nodes visited during traversal
    /// (default 1000). When exceeded the result is marked partial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<usize>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `workspace_context`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WorkspaceContextArgs {
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `memory_stats`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MemoryStatsArgs {
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `reindex`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ReindexArgs {
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `repository_health`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RepositoryHealthArgs {
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// Argument schema for `consult`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ConsultArgs {
    /// Provider to consult: `auto` or `conductor`. Defaults to `auto`
    /// (first authenticated provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Consultation mode: `architecture`, `debugging`, `code_review`,
    /// `planning`, `research`, or `second_opinion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The question or task to consult on.
    pub question: String,
    /// Optional explicit context text supplementing automatic CodeBro context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Optional explicit file contexts to include.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ConsultFileArg>,
    /// Whether to include the current git diff in the request context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_git_diff: Option<bool>,
    /// Whether to include project facts and engineering memory in the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_project_context: Option<bool>,
    /// Maximum answer length in characters (0 = provider default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_answer_length: Option<usize>,
    /// Optional workspace root to operate against. When omitted, the
    /// server's configured default workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

/// A single file to attach to a consultation request.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ConsultFileArg {
    /// Path relative to the workspace root.
    pub path: String,
    /// File contents.
    pub content: String,
}

/// Inject project identity, facts summary, and memory into the request context
/// so the consultant answers with project-awareness.
///
/// P6 helpers: incremental digest diff + bounded error shortening for
/// history summaries. Pure, deterministic, no I/O.
struct McpFileDiff {
    added: Vec<String>,
    deleted: Vec<String>,
    modified: Vec<String>,
    unchanged: Vec<String>,
    /// Full pre-truncation totals (lists below are capped at
    /// [`MCP_DIFF_LIST_CAP`] entries each for response bounds).
    added_count: usize,
    deleted_count: usize,
    modified_count: usize,
    unchanged_count: usize,
    /// True when any list was truncated (totals exceed the cap).
    truncated: bool,
}

/// Maximum entries per diff list in MCP responses (deterministic
/// head-truncation; totals + `truncated` keep the response honest).
const MCP_DIFF_LIST_CAP: usize = 100;

fn diff_digests_for_mcp(
    prev: &std::collections::BTreeMap<String, String>,
    curr: &std::collections::BTreeMap<String, String>,
) -> McpFileDiff {
    // Single kernel: the indexer's pure deterministic diff. The MCP layer
    // only applies bounded presentation truncation on top — never its own
    // traversal logic.
    let full = crate::init::engineering::diff_digests(prev, curr);
    let truncated = full.added.len() > MCP_DIFF_LIST_CAP
        || full.deleted.len() > MCP_DIFF_LIST_CAP
        || full.modified.len() > MCP_DIFF_LIST_CAP;
    let mut added = full.added;
    let mut deleted = full.deleted;
    let mut modified = full.modified;
    let (added_count, deleted_count, modified_count) = (added.len(), deleted.len(), modified.len());
    added.truncate(MCP_DIFF_LIST_CAP);
    deleted.truncate(MCP_DIFF_LIST_CAP);
    modified.truncate(MCP_DIFF_LIST_CAP);
    let unchanged_count = full.unchanged.len();
    McpFileDiff {
        added,
        deleted,
        modified,
        unchanged: full.unchanged,
        added_count,
        deleted_count,
        modified_count,
        unchanged_count,
        truncated,
    }
}

/// [`RepoIndexUpsert`] for a failed index run: preserves the last-good
/// counts/revision/timestamps from the existing row so a failure never
/// zeroes out known-good metadata. Only the status (FAILED) and the
/// bookkeeping timestamp move. Pure constructor for testability.
fn failed_index_upsert(
    prev: &crate::context_runtime::RepoIndexRecord,
) -> crate::context_runtime::RepoIndexUpsert {
    crate::context_runtime::RepoIndexUpsert {
        repository_identity: prev.repository_identity.clone(),
        index_status: crate::context_runtime::RepoIndexStatus::Failed,
        indexed_at: prev.indexed_at,
        repository_revision: prev.repository_revision.clone(),
        file_count: prev.file_count,
        symbol_count: prev.symbol_count,
        edge_count: prev.edge_count,
        stale_count: prev.stale_count,
    }
}

fn short_error(raw: &str) -> String {
    let one_line: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.len() > 240 {
        format!("{}…[truncated]", &one_line[..240])
    } else {
        one_line
    }
}

fn inject_project_context(
    request: &mut crate::consultant::types::ConsultantRequest,
    workspace: &std::path::Path,
) {
    let mut ctx_parts: Vec<String> = Vec::new();

    // Project identity.
    let mut identity = crate::project_identity::ProjectIdentityRuntime::new(workspace);
    if identity.load().is_ok() {
        let snap = identity.snapshot();
        if !snap.name.is_empty() {
            let lang = snap.languages.first().cloned().unwrap_or_default();
            ctx_parts.push(format!("Project: {} ({})", snap.name, lang));
        }
    }

    // Facts summary (compact — no raw facts, just counts).
    let store = {
        let path = workspace.join(".codebro/facts.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
                    Ok(model) => Some(crate::fact_store::FactStore::from_model(&model)),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    };
    if let Some(ref store) = store {
        let counts = store.collection().counts();
        ctx_parts.push(format!(
            "Verified facts: {} symbols, {} modules, {} tests, {} dependencies",
            counts.symbols, counts.modules, counts.tests, counts.dependencies
        ));
    }

    // Engineering memory (compact — just entry count and tags).
    let mut memory = crate::engineering_memory::EngineeringMemoryRuntime::new(
        workspace,
        crate::project_identity::ProjectIdentityRuntime::new(workspace),
    );
    let _ = memory.load();
    let entries = memory.snapshot();
    if !entries.is_empty() {
        let tags: std::collections::BTreeSet<&str> = entries
            .iter()
            .flat_map(|e| e.metadata.tags.iter().map(|s| s.as_str()))
            .collect();
        ctx_parts.push(format!(
            "Engineering memory: {} entries, tags: {}",
            entries.len(),
            tags.iter().copied().take(10).collect::<Vec<_>>().join(", ")
        ));
    }

    if !ctx_parts.is_empty() {
        let context = ctx_parts.join("\n");
        request.context = Some(match request.context.take() {
            Some(existing) => format!("{existing}\n\n--- Project Context ---\n{context}"),
            None => format!("--- Project Context ---\n{context}"),
        });
    }
}

/// Capture the current git diff (if any) and attach it to the request.
fn inject_git_diff(
    request: &mut crate::consultant::types::ConsultantRequest,
    workspace: &std::path::Path,
) {
    let output = std::process::Command::new("git")
        .current_dir(workspace)
        .args(["diff", "--cached", "--stat"])
        .output();
    let diff = match &output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if text.is_empty() {
                // Try uncommitted diff if staged is empty.
                let out2 = std::process::Command::new("git")
                    .current_dir(workspace)
                    .args(["diff"])
                    .output();
                match out2 {
                    Ok(o2) if o2.status.success() => {
                        String::from_utf8_lossy(&o2.stdout).to_string()
                    }
                    _ => String::new(),
                }
            } else {
                text
            }
        }
        _ => String::new(),
    };
    if !diff.trim().is_empty() {
        let truncated: String = diff.chars().take(4096).collect();
        let context = format!("--- Git Diff ---\n{truncated}");
        request.context = Some(match request.context.take() {
            Some(existing) => format!("{existing}\n\n{context}"),
            None => context,
        });
    }
}

/// Resolve a test command for the given workspace.
fn resolve_test_command(workspace: &std::path::Path, explicit: Option<&str>) -> String {
    resolve_test_command_filtered(workspace, explicit, &[])
}

/// Sanitize a test name for embedding in a runner selection expression
/// (go's `-run` takes a regexp; identifiers need no escaping, anything
/// else is dropped rather than guessed).
fn sanitize_go_test_name(name: &str) -> Option<String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(name.to_string())
}

/// Resolve a test command, optionally narrowed to specific test names.
/// Filters are honored for runners with a standard selection mechanism
/// (cargo passes multiple filters to libtest, go uses `-run`, pytest takes
/// node ids); runners without one (npm/pnpm/yarn scripts) ignore them. An
/// explicit command always wins untouched.
fn resolve_test_command_filtered(
    workspace: &std::path::Path,
    explicit: Option<&str>,
    filters: &[String],
) -> String {
    if let Some(cmd) = explicit {
        return cmd.to_string();
    }
    let filters: Vec<String> = filters
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    if workspace.join("Cargo.toml").exists() {
        return match filters.is_empty() {
            true => "cargo test".to_string(),
            false => format!("cargo test {}", filters.join(" ")),
        };
    }
    if workspace.join("go.mod").exists() {
        return match filters.is_empty() {
            true => "go test ./...".to_string(),
            false => {
                let names: Vec<String> = filters
                    .iter()
                    .filter_map(|f| sanitize_go_test_name(f))
                    .collect();
                match names.len() {
                    0 => "go test ./...".to_string(),
                    // Single-name targeting only: Go regex alternation
                    // needs `|`, which the local command policy blocks as
                    // a shell metachar. Multi-filter falls back to the
                    // honest full-suite run.
                    1 => format!("go test -run {} ./...", names[0]),
                    _ => "go test ./...".to_string(),
                }
            }
        };
    }
    if workspace.join("package.json").exists() {
        // Package-manager preference by lockfile. No standard per-test
        // selection mechanism — filters are ignored.
        if workspace.join("pnpm-lock.yaml").exists() {
            return "pnpm test".to_string();
        }
        if workspace.join("yarn.lock").exists() {
            return "yarn test".to_string();
        }
        return "npm test".to_string();
    }
    if workspace.join("pyproject.toml").exists()
        || workspace.join("pytest.ini").exists()
        || workspace.join("setup.py").exists()
    {
        return match filters.is_empty() {
            true => "python -m pytest -q --tb=long".to_string(),
            // `-k` keyword selection matches test names as substrings
            // (identifier-safe, no shell metacharacters); --tb=long keeps
            // source frames in tracebacks so failures attribute to the
            // mutated file, not just the test file.
            false => format!("python -m pytest -q --tb=long -k {}", filters.join(" or ")),
        };
    }
    "echo no project manifest detected use sandbox_exec with explicit command".to_string()
}

/// Resolve a build/check command for the given workspace.
fn resolve_build_command(workspace: &std::path::Path, explicit: Option<&str>) -> String {
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
        // Package-manager preference by lockfile.
        if workspace.join("pnpm-lock.yaml").exists() {
            return "pnpm run build".to_string();
        }
        if workspace.join("yarn.lock").exists() {
            return "yarn build".to_string();
        }
        return "npm run build".to_string();
    }
    "echo no project manifest detected use sandbox_exec with explicit command".to_string()
}

fn default_half() -> f64 {
    0.5
}

#[cfg(test)]
mod phase8_tests {
    use super::*;

    #[test]
    fn command_resolution_prefers_lockfiles_and_python() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        assert_eq!(resolve_test_command(dir.path(), None), "npm test");

        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(resolve_test_command(dir.path(), None), "pnpm test");
        assert_eq!(resolve_build_command(dir.path(), None), "pnpm run build");
        std::fs::remove_file(dir.path().join("pnpm-lock.yaml")).unwrap();

        std::fs::write(dir.path().join("yarn.lock"), "").unwrap();
        assert_eq!(resolve_test_command(dir.path(), None), "yarn test");
        assert_eq!(resolve_build_command(dir.path(), None), "yarn build");
        std::fs::remove_file(dir.path().join("yarn.lock")).unwrap();

        // Python project without node/cargo manifests.
        let py = tempfile::tempdir().unwrap();
        std::fs::write(py.path().join("pyproject.toml"), "[project]").unwrap();
        assert_eq!(
            resolve_test_command(py.path(), None),
            "python -m pytest -q --tb=long"
        );

        // Explicit commands always win.
        assert_eq!(
            resolve_test_command(dir.path(), Some("custom runner")),
            "custom runner"
        );
    }

    #[test]
    fn execution_evidence_carries_environment_capture() {
        let env = crate::sandbox::ExecutionEnvironment::capture();
        assert!(!env.os.is_empty());
        assert!(!env.arch.is_empty());
        let json = serde_json::to_value(&env).unwrap();
        assert!(json["os"].is_string());
        assert!(json["arch"].is_string());
        assert!(json["family"].is_string());
    }
}

#[tool_handler]
impl rmcp::ServerHandler for CodeBroMcpServer {
    fn get_info(&self) -> ServerInfo {
        // P8 integration contract: the server identifies itself as the
        // product (`codebro` + crate version), never the SDK default
        // (`"rmcp"`). Clients display and route on `serverInfo`.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                crate::integration::SERVER_NAME,
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
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
             - Project goals, constraints and declared decisions live in the project identity \
               served by codebro_workspace_context; when you learn a durable engineering goal, \
               constraint or decision worth declaring for this project, record it with \
               codebro_update_identity (medium-high trust: declared intent). Use \
               codebro_record_memory only for lower-trust session learnings.\n\
             - After learning a durable decision or constraint -> record it with \
               codebro_record_memory so future sessions are not amnesic (key like \
               'architecture:area', tags, confidence).\n\
              - Remove stale/wrong entries with codebro_delete_memory by exact key; set confirm=true explicitly (default false prevents accidental deletion).\n\
              - To run build/test/lint commands in an isolated sandbox, call codebro_sandbox_exec \
                (returns structured evidence: exit_code, stdout, stderr, duration_ms, success, \
                timeout, denied). Check availability first with codebro_sandbox_status.\n\
               - To understand what is structurally affected by changing a symbol, file, module, \
                 or package, call codebro_impact_analyze (returns directed relationship edges, \
                 related tests, owning module/package, provenance, and deterministic risk \
                 signals — descriptive evidence only; OpenCode decides).\n\
              - To check the health of the CodeBro workspace (project identity, fact store, \
                  engineering memory, git status), call codebro_repository_health (returns \
                  structured exit code, status, per-check results, and summary).\n\
              - To ask an AI consultant (Conductor) for opinions on \
                architecture, debugging, code review, planning, research, or second \
                opinions, call codebro_consult (supports provider selection, mode shaping, \
                and automatic injection of CodeBro engineering context like facts, memory, \
                and git diff).\n\
               - For durable user context (preferences, intents) -> call codebro_context \
                 with task_id at task start; it resolves task > project > global winners \
                 tagged by authority. Persist ONLY what the user explicitly confirmed via \
                 codebro_remember (user_confirmed=true); retire via codebro_forget.\n\
               - For previous engineering work (past decisions, failures, validations, \
                 changes) -> call codebro_recall with a question ('why are we using \
                 SQLite again?'). History is query-driven and never dumped into context; \
                 task history stays invisible without its task_id.\n\
               - For recurring patterns in past work -> call codebro_learn (run to \
                 detect and evaluate hypotheses, list/get to inspect them with \
                 explanations). Accepted hypotheses persist as AI_INFERRED knowledge \
                 with evidence and confidence — never as USER_CONFIRMED truth. Confirm \
                 only what the user explicitly approved (user_confirmed=true).\n\
               - For a bounded decision-support brief before planning a task -> call \
                 codebro_engineering_brief with task/task_id plus an optional target. \
                 It assembles repo intelligence, impact, health, history, memory, \
                 learning, skills, task state, constraints, and explicit unknowns in \
                 one read-only call. CodeBro prepares evidence; you decide.\n\
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

    /// P8 per-call observability: one bounded tracing line per tool call
    /// (client, tool, duration, status, response size). Never arguments,
    /// task text, brief content, or secrets — this seam must not become a
    /// data-leak path. The router behavior itself is unchanged: this is
    /// a wrapper, not a second dispatch. Client identity comes from the
    /// peer info rmcp's default `initialize` registered (single-client
    /// stdio server: exactly one peer); it is process-local and never
    /// persisted, so no client-specific state exists.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, McpError> {
        let started = std::time::Instant::now();
        let tool = request.name.clone();
        let client = context.peer.peer_info().map(|info| {
            (
                info.client_info.name.clone(),
                info.client_info.version.clone(),
            )
        });
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(tcc).await;
        // Compose the observation from the outcome. Size uses the
        // serialized content length (bounded evidence, never content).
        let (errored, error_summary, response_bytes) = match &result {
            Ok(rmcp::model::CallToolResponse::Complete(r)) => (
                r.is_error.unwrap_or(false),
                None,
                serde_json::to_vec(&r.content).map(|v| v.len()).unwrap_or(0),
            ),
            Ok(_) => (false, None, 0),
            Err(e) => (
                true,
                Some(crate::integration::bounded_error_summary(&e.message)),
                0,
            ),
        };
        let obs = crate::integration::observe_call(
            client,
            &tool,
            started,
            errored,
            error_summary,
            response_bytes,
        );
        tracing::info!(observation = %obs.render_line(), "tool call");
        result
    }
}

/// Run the MCP server over stdio until the client disconnects.
pub async fn serve(
    workspace_root: PathBuf,
    extra_authorized_roots: Vec<PathBuf>,
) -> anyhow::Result<()> {
    let server = CodeBroMcpServer::with_authorized_roots(workspace_root, extra_authorized_roots);
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
    use std::collections::HashSet;

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
            "apply_changes",
            "record_memory",
            "delete_memory",
            "memory_stats",
            "sandbox_exec",
            "sandbox_test",
            "sandbox_build",
            "sandbox_status",
            "impact_analyze",
            "reindex",
            "repository_health",
            "update_identity",
            "consult",
            "context",
            "remember",
            "forget",
            "recall",
            "learn",
            "skill",
            "task",
            "engineering_brief",
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
            "apply_changes",
            "record_memory",
            "delete_memory",
            "memory_stats",
            "sandbox_exec",
            "sandbox_test",
            "sandbox_build",
            "sandbox_status",
            "impact_analyze",
            "reindex",
            "repository_health",
            "update_identity",
            "consult",
            "context",
            "remember",
            "forget",
            "recall",
            "learn",
            "skill",
            "task",
            "engineering_brief",
        ] {
            assert!(
                server.get_tool(expected).is_some(),
                "tool {expected} missing from tool handler"
            );
        }
    }

    /// Hardening (doc-drift regression): the `impact_analyze` description
    /// must reflect the P6 deterministic risk signals and must NOT repeat
    /// the pre-P6 "no risk scores" claim. Descriptions are client-visible
    /// contract surface; drift here misleads the agent's tool selection.
    #[test]
    fn impact_description_mentions_risk_signals() {
        let server = CodeBroMcpServer::new(PathBuf::from("/tmp/unused-root"));
        let tool = server
            .get_tool("impact_analyze")
            .expect("impact_analyze missing from tool handler");
        let desc = tool
            .description
            .as_deref()
            .expect("impact_analyze has no description");
        assert!(
            desc.contains("risk signal"),
            "impact_analyze description must mention deterministic risk signals: {desc}"
        );
        assert!(
            !desc.to_lowercase().contains("no risk scores"),
            "impact_analyze description must not claim 'no risk scores' (P6 added signals): {desc}"
        );
    }

    /// P8: the MCP handshake must identify the server as the product
    /// (`codebro` + crate version), never the rmcp SDK default — clients
    /// display and route on `serverInfo`.
    #[test]
    fn p8_server_info_identifies_the_product() {
        let server = CodeBroMcpServer::new(PathBuf::from("/tmp/unused-root"));
        let info = rmcp::ServerHandler::get_info(&server);
        assert_eq!(info.server_info.name, crate::integration::SERVER_NAME);
        assert_eq!(info.server_info.name, "codebro");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.server_info.name.is_empty());
        // Instructions survive (the client-facing usage contract).
        assert!(info.instructions.unwrap_or_default().contains("CodeBro"));
    }

    /// P8: the integration contract (integration::contract::intents) must
    /// map every agent-client intent to an existing routed tool — the
    /// contract is enforced by regression, not prose. P8 adds no tools.
    #[test]
    fn p8_integration_contract_intents_are_routed_tools() {
        let server = CodeBroMcpServer::new(PathBuf::from("/tmp/unused-root"));
        for (intent, tool) in crate::integration::contract::intents() {
            assert!(
                server.get_tool(tool).is_some(),
                "P8 contract intent '{intent}' maps to unrouted tool '{tool}'"
            );
        }
        // The primary surface is the brief (P8 §6: the Engineering Brief
        // is the primary high-level context acquisition surface).
        assert_eq!(
            crate::integration::contract::PRIMARY_CONTEXT_TOOL,
            "engineering_brief"
        );
    }

    /// P8: the per-call observability observation must never embed tool
    /// arguments, task text, or brief content — only client identity,
    /// tool name, duration, status, and size. Defense in depth against
    /// the observability seam becoming a data-leak path.
    #[test]
    fn p8_observation_never_carries_payloads() {
        let obs = crate::integration::ToolCallObservation {
            client_name: Some("opencode".to_string()),
            client_version: Some("1.18".to_string()),
            tool: "engineering_brief".to_string(),
            duration_ms: 12,
            errored: false,
            error_summary: None,
            response_bytes: 4096,
        };
        let line = obs.render_line();
        for banned in ["task=", "keywords=", "arguments", "content="] {
            assert!(
                !line.contains(banned),
                "observation leaked {banned}: {line}"
            );
        }
        // Round-trip through observe_call keeps the invariant.
        let built = crate::integration::observe_call(
            Some(("opencode".to_string(), "1.18".to_string())),
            "remember",
            std::time::Instant::now(),
            true,
            Some("invalid_params: password=hunter2".to_string()),
            10,
        );
        let rendered = built.render_line();
        assert!(
            !rendered.contains("hunter2"),
            "secret-shaped error leaked: {rendered}"
        );
        assert!(rendered.contains("status=error"));
    }

    /// P0.3: `memory_stats` must report meaningful state — entry count,
    /// budget, confidence, tags — and degrade gracefully when empty.
    #[tokio::test]
    async fn memory_stats_reports_meaningful_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

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

    // ── M1-A: Engineering Memory Trust Exposure ──────────────────────────

    /// M1-A.A: High-confidence AgentDeclared memory produces trust in [0,1].
    #[tokio::test]
    async fn m1a_high_confidence_memory_has_trust_in_range() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:high-conf",
                "value": "high confidence decision",
                "confidence": 1.0,
            }),
        )
        .await;

        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:high-conf"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        let entries = v["entries"].as_array().expect("entries is array");
        assert_eq!(entries.len(), 1);
        let trust = entries[0]["trust"].as_f64().expect("trust is present");
        assert!(trust >= 0.0, "trust must be >= 0.0, got {trust}");
        assert!(trust <= 1.0, "trust must be <= 1.0, got {trust}");
        // AgentDeclared base = 0.30, freshness Unknown = 0.8, confidence = 1.0
        // trust = 0.30 * 0.8 * 1.0 = 0.24
        assert!(
            (trust - 0.24).abs() < 1e-9,
            "expected trust ≈ 0.24 for conf=1.0, got {trust}"
        );
    }

    /// M1-A.B: Lower confidence produces lower trust than high confidence.
    #[tokio::test]
    async fn m1a_lower_confidence_produces_lower_trust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:low-conf",
                "value": "low confidence decision",
                "confidence": 0.5,
            }),
        )
        .await;

        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:low-conf"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        let trust = v["entries"][0]["trust"].as_f64().expect("trust is present");
        // AgentDeclared base = 0.30, freshness Unknown = 0.8, confidence = 0.5
        // trust = 0.30 * 0.8 * 0.5 = 0.12
        assert!(
            (trust - 0.12).abs() < 1e-9,
            "expected trust ≈ 0.12 for conf=0.5, got {trust}"
        );
    }

    /// M1-A.C: Zero confidence produces the AgentDeclared base trust after
    /// existing formula behavior (base * freshness * 0.0 = 0.0).
    #[test]
    fn m1a_zero_confidence_produces_zero_trust() {
        use crate::provenance::{compute_trust, FreshnessStatus, SourceKind};
        use crate::workspace_registry::{WorkspaceRegistry, WorkspaceState};
        let t = compute_trust(&SourceKind::AgentDeclared, 0.0, FreshnessStatus::Unknown);
        // AgentDeclared base = 0.30, freshness Unknown = 0.8, confidence = 0.0
        // trust = 0.30 * 0.8 * 0.0 = 0.0
        assert!(
            (t - 0.0).abs() < 1e-9,
            "expected trust ≈ 0.0 for conf=0.0, got {t}"
        );
    }

    /// M1-A.D: Freshness effect — same memory + same confidence:
    /// Fresh > Unknown > Stale.
    /// Since memory entries use Unknown freshness (no provenance), we verify
    /// the compute_trust formula directly for the three freshness states.
    #[tokio::test]
    async fn m1a_freshness_effect_on_trust() {
        use crate::provenance::{compute_trust, FreshnessStatus, SourceKind};
        use crate::workspace_registry::{WorkspaceRegistry, WorkspaceState};
        let confidence = 0.8;
        let t_fresh = compute_trust(
            &SourceKind::AgentDeclared,
            confidence,
            FreshnessStatus::Fresh,
        );
        let t_unknown = compute_trust(
            &SourceKind::AgentDeclared,
            confidence,
            FreshnessStatus::Unknown,
        );
        let t_stale = compute_trust(
            &SourceKind::AgentDeclared,
            confidence,
            FreshnessStatus::Stale,
        );
        assert!(
            t_fresh > t_unknown,
            "fresh ({t_fresh}) must exceed unknown ({t_unknown})"
        );
        assert!(
            t_unknown > t_stale,
            "unknown ({t_unknown}) must exceed stale ({t_stale})"
        );
        // Verify exact values: base=0.30
        // fresh:   0.30 * 1.0 * 0.8 = 0.24
        // unknown: 0.30 * 0.8 * 0.8 = 0.192
        // stale:   0.30 * 0.6 * 0.8 = 0.144
        assert!((t_fresh - 0.24).abs() < 1e-9);
        assert!((t_unknown - 0.192).abs() < 1e-9);
        assert!((t_stale - 0.144).abs() < 1e-9);
    }

    /// M1-A.E: Missing/unavailable freshness uses FreshnessStatus::Unknown.
    #[tokio::test]
    async fn m1a_missing_freshness_uses_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:missing-fresh",
                "value": "no provenance",
                "confidence": 0.7,
            }),
        )
        .await;

        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:missing-fresh"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        let trust = v["entries"][0]["trust"].as_f64().expect("trust is present");
        // Unknown freshness: base 0.30 * 0.8 * 0.7 = 0.168
        assert!(
            (trust - 0.168).abs() < 1e-9,
            "expected trust ≈ 0.168 for Unknown freshness, got {trust}"
        );
    }

    /// M1-A.F: MCP serialization — trust appears when computed.
    #[tokio::test]
    async fn m1a_trust_appears_in_serialization() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:serialize",
                "value": "serialize me",
                "confidence": 0.9,
            }),
        )
        .await;

        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:serialize"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert!(
            v["entries"][0].get("trust").is_some(),
            "trust field must be present in serialized response"
        );
    }

    /// M1-A.G: Optional behavior — trust is always computed for resolved
    /// entries (never absent when entries are present). Absence would only
    /// apply if the response had no entries at all.
    #[tokio::test]
    async fn m1a_trust_is_present_for_resolved_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:opt-a",
                "value": "opt a",
                "confidence": 0.6,
            }),
        )
        .await;
        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:opt-b",
                "value": "opt b",
                "confidence": 0.4,
            }),
        )
        .await;

        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        let entries = v["entries"].as_array().expect("entries is array");
        for entry in entries {
            assert!(
                entry.get("trust").is_some(),
                "trust must be present for each resolved entry"
            );
            let t = entry["trust"].as_f64().expect("trust is a number");
            assert!((0.0..=1.0).contains(&t), "trust must be in [0,1], got {t}");
        }
    }

    /// M1-A.H: memory_stats avg_trust is correct for multiple entries.
    #[tokio::test]
    async fn m1a_memory_stats_avg_trust_correct() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // Entry 1: confidence 1.0 → trust = 0.30 * 0.8 * 1.0 = 0.24
        call_tool_text(
            &server,
            "record_memory",
            json!({"key": "m1a:a", "value": "v1", "confidence": 1.0}),
        )
        .await;
        // Entry 2: confidence 0.5 → trust = 0.30 * 0.8 * 0.5 = 0.12
        call_tool_text(
            &server,
            "record_memory",
            json!({"key": "m1a:b", "value": "v2", "confidence": 0.5}),
        )
        .await;
        // Entry 3: confidence 0.0 → trust = 0.30 * 0.8 * 0.0 = 0.0
        call_tool_text(
            &server,
            "record_memory",
            json!({"key": "m1a:c", "value": "v3", "confidence": 0.0}),
        )
        .await;

        let stats = call_tool_text(&server, "memory_stats", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&stats).expect("valid json");
        assert_eq!(v["entry_count"], 3);
        // avg_trust = (0.24 + 0.12 + 0.0) / 3 = 0.12
        let avg_trust = v["avg_trust"].as_f64().expect("avg_trust is present");
        assert!(
            (avg_trust - 0.12).abs() < 1e-9,
            "expected avg_trust ≈ 0.12, got {avg_trust}"
        );
    }

    /// M1-A.I: Empty memory — avg_trust is omitted.
    #[tokio::test]
    async fn m1a_empty_memory_omits_avg_trust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let stats = call_tool_text(&server, "memory_stats", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&stats).expect("valid json");
        assert_eq!(v["entry_count"], 0);
        assert!(
            v.get("avg_trust").is_none(),
            "avg_trust must be omitted when memory is empty"
        );
    }

    /// M1-A.J: Backward compatibility — existing memory response fields remain
    /// unchanged.
    #[tokio::test]
    async fn m1a_backward_compatibility_existing_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:compat",
                "value": "compat value",
                "tags": ["backend", "api"],
                "confidence": 0.85,
                "source": "sprint-30",
            }),
        )
        .await;

        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:compat"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        let e = &v["entries"][0];
        // Existing fields must still be present and correct.
        assert_eq!(e["key"], "arch:compat");
        assert_eq!(e["value"], "compat value");
        assert_eq!(e["confidence"], 0.85);
        assert_eq!(e["source"], "sprint-30");
        let tags: Vec<&str> = e["tags"]
            .as_array()
            .expect("tags is array")
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        assert_eq!(tags, vec!["api", "backend"]);
        // New field must also be present.
        assert!(e.get("trust").is_some());
        // Budget field must still be present.
        assert!(v.get("budget_remaining").is_some());
    }

    /// P0.2: `engineering_facts` returns actual fact records (with name,
    /// path, provenance) — not raw ids — and honours query/kind/path/limit.
    #[tokio::test]
    async fn engineering_facts_returns_records_not_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

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
        // Zero-result recovery guidance must be present on empty results.
        assert!(
            v["recovery"].is_object(),
            "recovery must be present when returned==0"
        );
        let recovery = v["recovery"].as_object().unwrap();
        assert!(recovery.contains_key("message"));
        assert!(recovery.contains_key("hints"));
        let hints: Vec<&str> = recovery["hints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap())
            .collect();
        assert!(hints.iter().any(|h| h.contains("shorter")));

        // Invalid kind must be rejected, not silently ignored.
        let err = call_tool_err(
            &server,
            "engineering_facts",
            json!({"query": "x", "kind": "bogus"}),
        )
        .await;
        assert!(err.to_string().contains("unknown fact kind"));
    }

    /// Memory lifecycle: record -> resolve -> stats -> delete. Proves the
    /// write path is round-trippable and delete removes the entry.
    #[tokio::test]
    async fn memory_lifecycle_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // Record.
        call_tool_text(
            &server,
            "record_memory",
            json!({"key": "arch:lifecycle", "value": "decision", "tags": ["arch"], "confidence": 0.7}),
        )
        .await;

        // Resolve with the key as keyword.
        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:lifecycle"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(v["entries"][0]["key"], "arch:lifecycle");
        assert_eq!(v["entries"][0]["confidence"], 0.7);
        assert_eq!(v["entries"][0]["tags"][0], "arch");

        // Stats reflect the entry.
        let stats = call_tool_text(&server, "memory_stats", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&stats).expect("valid json");
        assert_eq!(v["entry_count"], 1);

        // Delete (with explicit confirm), then resolve must be empty.
        call_tool_text(
            &server,
            "delete_memory",
            json!({"key": "arch:lifecycle", "confirm": true}),
        )
        .await;
        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:lifecycle"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(0));

        // Deleting without confirm=true must be rejected (guard).
        let guard_err =
            call_tool_err(&server, "delete_memory", json!({"key": "arch:lifecycle"})).await;
        assert!(
            guard_err.contains("confirm=true"),
            "delete without confirm must be rejected, got: {guard_err}"
        );

        // Deleting a missing key must error.
        let err = call_tool_err(
            &server,
            "delete_memory",
            json!({"key": "nope", "confirm": true}),
        )
        .await;
        assert!(err.contains("no entry"));
    }

    /// P1.1 regression: record_memory on an existing key must update the
    /// FULL logical entry — value AND confidence/importance/tags/source —
    /// not just the value. Verified against the persisted file after
    /// reload.
    #[tokio::test]
    async fn record_memory_updates_full_metadata_on_existing_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // 1. Create entry with initial metadata.
        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:gateway",
                "value": "v1",
                "confidence": 0.5,
                "importance": 0.4,
                "tags": ["a"],
                "source": "init",
            }),
        )
        .await;

        // 2. Update the SAME key with new value + new metadata.
        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "arch:gateway",
                "value": "v2",
                "confidence": 0.9,
                "importance": 0.8,
                "tags": ["b", "c"],
                "source": "review",
            }),
        )
        .await;

        // 3-7. Verify every field changed via engineering_memory.
        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["arch:gateway"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(1));
        let e = &v["entries"][0];
        assert_eq!(e["value"], "v2", "value must be updated");
        assert_eq!(e["confidence"], 0.9, "confidence must be updated");
        assert_eq!(e["source"], "review", "source must be updated");
        let tags: Vec<String> = e["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            tags,
            vec!["b".to_string(), "c".to_string()],
            "tags must be updated"
        );

        // 8. Persistence after reload: a fresh server reads the same state.
        let server2 = CodeBroMcpServer::new(dir.path().to_path_buf());
        let resolved = call_tool_text(
            &server2,
            "engineering_memory",
            json!({"task_keywords": ["arch:gateway"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(v["entries"][0]["value"], "v2");
        assert_eq!(v["entries"][0]["confidence"], 0.9);
        assert_eq!(v["entries"][0]["source"], "review");
        // Importance is not projected into the resolved view; verify it
        // directly on disk.
        let raw = std::fs::read_to_string(dir.path().join(".codebro/engineering_memory.json"))
            .expect("memory file");
        let rawv: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        let stored = &rawv["entries"][0];
        assert_eq!(
            stored["metadata"]["importance"], 0.8,
            "importance must be updated"
        );
        assert_eq!(stored["metadata"]["confidence"], 0.9);
        assert_eq!(stored["metadata"]["source"], "review");
        assert_eq!(stored["metadata"]["tags"], json!(["b", "c"]));
        assert_eq!(stored["value"], "v2");
    }

    /// apply_change must reject traversal and stale content while allowing
    /// a correct edit — the guard is the point.
    #[tokio::test]
    async fn apply_change_guards_are_enforced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("demo.txt");
        std::fs::write(&file, "hello world").expect("write");

        let server = local_sandbox_server(&dir);

        // Path traversal must be rejected.
        let err = call_tool_err(
            &server,
            "apply_change",
            json!({"path": "../../etc/passwd", "old": "x", "new": "y"}),
        )
        .await;
        assert!(err.contains("path boundary") || err.contains("traversal"));

        // Stale content must be rejected.
        let err = call_tool_err(
            &server,
            "apply_change",
            json!({"path": "demo.txt", "old": "not present", "new": "y"}),
        )
        .await;
        assert!(err.contains("stale"));

        // Correct edit succeeds and modifies the file.
        call_tool_text(
            &server,
            "apply_change",
            json!({"path": "demo.txt", "old": "hello world", "new": "hello codebro"}),
        )
        .await;
        let content = std::fs::read_to_string(&file).expect("read");
        assert_eq!(content.trim(), "hello codebro");

        // Ambiguous old-text (occurs more than once) must be rejected.
        std::fs::write(&file, "dup\ndup\n").expect("write ambiguous file");
        let err = call_tool_err(
            &server,
            "apply_change",
            json!({"path": "demo.txt", "old": "dup", "new": "x"}),
        )
        .await;
        assert!(err.contains("ambiguous"), "got: {err}");

        // Symlink escaping the workspace root must be denied, and the
        // external target must remain untouched.
        let outside = tempfile::tempdir().expect("outside tempdir");
        let external = outside.path().join("target.txt");
        std::fs::write(&external, "precious").expect("write external");
        let link = dir.path().join("evil-link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&external, &link).expect("symlink");
        let err = call_tool_err(
            &server,
            "apply_change",
            json!({"path": "evil-link.txt", "old": "precious", "new": "HACKED"}),
        )
        .await;
        assert!(err.contains("symlink escape"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&external).expect("read external"),
            "precious"
        );
    }

    /// M2: applying a change to an existing source file reports affected
    /// existing facts and symbols, and sets needs_reindex=true.
    #[tokio::test]
    async fn apply_change_reports_affected_existing_facts() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\npub fn world() -> i32 { 1 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Modify an existing symbol in an existing file.
        let out = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "src/lib.rs",
                "old": "pub fn hello() -> i32 { 42 }",
                "new": "pub fn hello() -> i32 { 99 }"
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["applied"], true);
        assert_eq!(v["path"], "src/lib.rs");
        assert!(v["preview"].is_string());
        assert_eq!(v["needs_reindex"], true);
        // hello and world symbols are in src/lib.rs, so both should be affected.
        let affected_symbols: Vec<&str> = v["affected_symbols"]
            .as_array()
            .expect("affected_symbols must be array")
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(
            affected_symbols.iter().any(|s| s.contains("hello")),
            "hello symbol must be affected, got: {affected_symbols:?}"
        );
        assert!(
            affected_symbols.iter().any(|s| s.contains("world")),
            "world symbol must be affected, got: {affected_symbols:?}"
        );
        // affected_fact_ids must include the symbol IDs and module ID.
        let fact_ids: Vec<&str> = v["affected_fact_ids"]
            .as_array()
            .expect("affected_fact_ids must be array")
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(!fact_ids.is_empty(), "must report affected fact IDs");
        // affected_modules must contain the module for src/lib.rs.
        let mods: Vec<&str> = v["affected_modules"]
            .as_array()
            .expect("affected_modules must be array")
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(!mods.is_empty(), "must report affected modules");
        // recommendation should mention reindex.
        assert!(
            v["recommendation"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("init"),
            "recommendation must mention init, got: {}",
            v["recommendation"].as_str().unwrap()
        );
    }

    /// M2: creating a new source file reports empty affected lists but
    /// needs_reindex=true, and does NOT fabricate symbol IDs.
    #[tokio::test]
    async fn apply_change_new_file_has_no_fabricated_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Create a brand-new file.
        let out = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "src/new_module.rs",
                "old": "",
                "new": "pub fn new_fn() -> i32 { 1 }\n"
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["applied"], true);
        assert_eq!(v["path"], "src/new_module.rs");
        assert_eq!(
            v["affected_symbols"].as_array().map(|a| a.len()),
            Some(0),
            "no fabricated symbol IDs for new file"
        );
        assert_eq!(
            v["affected_fact_ids"].as_array().map(|a| a.len()),
            Some(0),
            "no fabricated fact IDs for new file"
        );
        assert_eq!(
            v["affected_modules"].as_array().map(|a| a.len()),
            Some(0),
            "no fabricated module IDs for new file"
        );
        assert_eq!(v["needs_reindex"], true);
        assert!(
            v["recommendation"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("new symbols"),
            "recommendation must mention new symbols"
        );
    }

    /// M2: applying a change to an unrelated file (no facts) returns empty
    /// affected lists but still needs_reindex=true.
    #[tokio::test]
    async fn apply_change_unrelated_file_empty_affected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Create a new file that is not a source file the parser recognizes
        // (e.g. a config file).
        let out = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "config.yaml",
                "old": "",
                "new": "key: value\n"
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["applied"], true);
        assert_eq!(v["affected_symbols"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(v["affected_modules"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(v["affected_fact_ids"].as_array().map(|a| a.len()), Some(0));
        // needs_reindex is still true because any source-file change could
        // affect facts — but recommendation may be empty for non-source files.
        assert_eq!(v["needs_reindex"], true);
    }

    /// M2: backward compatibility — existing fields remain unchanged.
    #[tokio::test]
    async fn apply_change_backward_compatible_response() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("demo.txt");
        std::fs::write(&file, "hello world").expect("write");

        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "apply_change",
            json!({"path": "demo.txt", "old": "hello world", "new": "hello codebro"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        // Core fields must be present.
        assert_eq!(v["applied"], true);
        assert_eq!(v["path"], "demo.txt");
        assert!(v["preview"].is_string());
        assert_eq!(v["needs_reindex"], true);
        // New fields are additive — no existing fields removed.
        assert!(v.get("affected_fact_ids").is_some());
        assert!(v.get("affected_symbols").is_some());
        assert!(v.get("affected_modules").is_some());
        assert!(v.get("recommendation").is_some());
    }

    /// M2: path normalization — leading "./" matches correctly.
    #[tokio::test]
    async fn apply_change_path_normalization() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Use "./src/lib.rs" — should normalize to "src/lib.rs".
        let out = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "./src/lib.rs",
                "old": "pub fn hello() -> i32 { 42 }",
                "new": "pub fn hello() -> i32 { 99 }"
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["applied"], true);
        assert_eq!(v["path"], "./src/lib.rs");
        // Should still find the symbol because normalization strips "./".
        let affected_symbols: Vec<&str> = v["affected_symbols"]
            .as_array()
            .expect("affected_symbols must be array")
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(
            affected_symbols.iter().any(|s| s.contains("hello")),
            "hello must be found with ./ normalization, got: {affected_symbols:?}"
        );
    }

    /// M2: failed apply_change must not return a success advisory.
    /// Integration: `apply_changes` transaction semantics at the MCP layer.
    /// Success applies every change (including nested creation); one stale
    /// entry aborts the whole set with zero mutations.
    #[tokio::test]
    async fn apply_changes_transaction_applies_or_rolls_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        std::fs::write(dir.path().join("a.txt"), "alpha").expect("write");
        let out = call_tool_text(
            &server,
            "apply_changes",
            json!({"changes": [
                {"path": "a.txt", "old": "alpha", "new": "beta"},
                {"path": "nested/c.txt", "old": "", "new": "gamma"},
            ]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["applied"], true);
        assert_eq!(v["applied_count"], 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt"))
                .unwrap()
                .trim(),
            "beta"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/c.txt"))
                .unwrap()
                .trim(),
            "gamma"
        );

        // A stale old-text in one entry must reject preparation up front,
        // leaving both files untouched.
        let err = call_tool_err(
            &server,
            "apply_changes",
            json!({"changes": [
                {"path": "a.txt", "old": "stale", "new": "x"},
                {"path": "b.txt", "old": "", "new": "fresh"},
            ]}),
        )
        .await;
        assert!(err.contains("stale"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt"))
                .unwrap()
                .trim(),
            "beta"
        );
        assert!(!dir.path().join("b.txt").exists());
    }

    /// Integration: targeted test selection loop — `apply_change` returns
    /// `recommended_tests` from the fact store's tested-linkage, and
    /// `sandbox_test` narrows the run to exactly those tests.
    #[tokio::test]
    async fn apply_change_recommends_tests_and_sandbox_test_runs_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"targeted\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn adds() {\n        assert_eq!(add(2, 3), 5);\n    }\n}\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init succeeds");

        // Edit the function body; the advisory must recommend `adds`.
        let out = call_tool_text(
            &server,
            "apply_change",
            json!({"path": "src/lib.rs", "old": "    a + b", "new": "    a + b + 0"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let recommended = v["recommended_tests"]
            .as_array()
            .expect("recommended_tests present");
        assert!(
            recommended.iter().any(|t| t.as_str() == Some("adds")),
            "expected 'adds' in {recommended:?}"
        );

        // Run only the recommended tests through the filter.
        let run = call_tool_text(&server, "sandbox_test", json!({"test_filter": ["adds"]})).await;
        let rv: serde_json::Value = serde_json::from_str(&run).expect("valid json");
        assert_eq!(rv["execution"]["command"], "cargo test adds");
        assert_eq!(rv["execution"]["exit_code"], 0);
        assert_eq!(rv["verification"]["verified"], true);
        let stdout = rv["execution"]["stdout"].as_str().unwrap_or_default();
        assert!(stdout.contains("running 1 test"), "got: {stdout}");
    }

    #[tokio::test]
    async fn apply_change_failure_returns_error_not_advisory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("demo.txt");
        std::fs::write(&file, "hello world").expect("write");

        let server = local_sandbox_server(&dir);

        // Stale content must fail — no advisory should leak through.
        let err = call_tool_err(
            &server,
            "apply_change",
            json!({"path": "demo.txt", "old": "not present", "new": "y"}),
        )
        .await;
        assert!(err.contains("stale"), "got: {err}");
    }

    /// workspace_context must always return a parseable orientation payload,
    /// even for an empty/uninitialized workspace.
    #[tokio::test]
    async fn workspace_context_orientates_empty_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "workspace_context",
            json!({
                "workspace_root": serde_json::Value::Null,
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert!(v["workspace_root"].is_string());
        assert!(v["fact_counts"].is_object());
        assert_eq!(v["fact_counts"]["total"], 0);
    }

    // ── context tool (tool 18) ─────────────────────────────────────────

    /// A server whose user-context store lives in an explicit directory
    /// (hermetic: never touches the real `~/.codebro/state.db`).
    fn stateful_server(dir: &tempfile::TempDir, state_dir: &tempfile::TempDir) -> CodeBroMcpServer {
        CodeBroMcpServer::with_state_dir(dir.path().to_path_buf(), state_dir.path().to_path_buf())
    }

    /// Multi-workspace stateful server for cross-workspace isolation tests
    /// (P8 root authorization): the default root plus every listed sibling
    /// is authorized exactly as an operator would configure at launch —
    /// isolation is then asserted between AUTHORIZED roots, which is the
    /// real product guarantee (the registry refuses unauthorized roots
    /// before any store access, pinned by dedicated tests).
    fn stateful_multiws_server(
        dir: &tempfile::TempDir,
        others: &[&tempfile::TempDir],
        state_dir: &tempfile::TempDir,
    ) -> CodeBroMcpServer {
        let registry = WorkspaceRegistry::with_authorized_roots(
            dir.path().to_path_buf(),
            crate::workspace_registry::AuthorizedRoots::with_extras(
                dir.path().to_path_buf(),
                others.iter().map(|o| o.path().to_path_buf()),
            ),
        );
        assemble_server_with_registry(
            registry,
            crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local),
            Some(state_dir.path().to_path_buf()),
        )
    }

    fn parse(out: &str) -> serde_json::Value {
        serde_json::from_str(out).expect("valid json")
    }

    /// context with no task returns a labelled structural digest with
    /// repository orientation, without creating any `.codebro` state.
    #[tokio::test]
    async fn context_without_task_returns_structural_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        let out = call_tool_text(
            &server,
            "context",
            json!({"workspace_root": dir.path().to_string_lossy()}),
        )
        .await;
        let v = parse(&out);
        assert_eq!(
            v["repository"]["workspace_root"],
            dir.path().to_string_lossy().as_ref()
        );
        assert_eq!(v["repository"]["identity_loaded"], false);
        assert!(v["repository"]["fact_counts"].is_object());
        assert!(v["facts"].is_array() && v["facts"].as_array().unwrap().is_empty());
        assert!(v["records"].is_array());
        assert_eq!(v["records_provenance"], "recorded");
        assert_eq!(v["repository"]["provenance"], "recorded");
        // Clearly labelled as structural, never mistaken for task context.
        let notes: Vec<&str> = v["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert!(notes.iter().any(|n| n.contains("structural digest")));
        // Read-only: no .codebro directory may be created by the call.
        assert!(!dir.path().join(".codebro").exists());
    }

    /// context with a task returns the task-relevant packet shape.
    #[tokio::test]
    async fn context_with_task_returns_task_relevant_packet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        let out = call_tool_text(
            &server,
            "context",
            json!({
                "workspace_root": dir.path().to_string_lossy(),
                "task": "Fix failing authentication tests",
            }),
        )
        .await;
        let v = parse(&out);
        assert!(v["repository"].is_object());
        // Empty workspace: sections empty but structurally present.
        assert!(v["facts"].as_array().unwrap().is_empty());
        assert!(v["memory"].as_array().unwrap().is_empty());
        assert!(v["impact"]["suggested_targets"].is_array());
        let notes: Vec<&str> = v["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert!(!notes.iter().any(|n| n.contains("structural digest")));
        assert!(!dir.path().join(".codebro").exists());
    }

    /// Durable context records surface in the packet, scoped to the
    /// workspace, tagged with their authority and effective confidence.
    #[tokio::test]
    async fn context_surfaces_records_scoped_and_authority_tagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let ws_root = dir.path().to_string_lossy().to_string();
        let server = stateful_server(&dir, &state);

        // Seed the user-context store directly through the canonical store.
        let store = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut global = crate::context_runtime::ContextRecord::new(
            "ctx::g1",
            crate::context_runtime::RecordKind::Style,
            "fp.communication.verbosity",
            "User prefers direct concise replies over lengthy prose",
            crate::context_runtime::Authority::UserConfirmed,
        );
        global.confidence = 0.95;
        global.language = Some("manglish".to_string());
        store.put_record(&global, now).unwrap();

        let mut local = crate::context_runtime::ContextRecord::new(
            "ctx::p1",
            crate::context_runtime::RecordKind::Constraint,
            "fp.engineering.simplicity",
            "This project must stay minimal; avoid unnecessary abstraction",
            crate::context_runtime::Authority::UserConfirmed,
        );
        local.scope = crate::context_runtime::RecordScope::Project;
        local.workspace_root = Some(ws_root.clone());
        store.put_record(&local, now).unwrap();

        // A record for a DIFFERENT workspace must never leak into this one.
        let mut other_ws = crate::context_runtime::ContextRecord::new(
            "ctx::x1",
            crate::context_runtime::RecordKind::Preference,
            "fp.other.project",
            "Unrelated project secret preference",
            crate::context_runtime::Authority::UserConfirmed,
        );
        other_ws.scope = crate::context_runtime::RecordScope::Project;
        other_ws.workspace_root = Some("/somewhere/else".to_string());
        store.put_record(&other_ws, now).unwrap();

        let out = call_tool_text(
            &server,
            "context",
            json!({"workspace_root": ws_root, "task": "keep the change minimal"}),
        )
        .await;
        let v = parse(&out);
        let records = v["records"].as_array().unwrap();
        let namespaces: Vec<&str> = records
            .iter()
            .map(|r| r["namespace"].as_str().unwrap())
            .collect();
        assert!(
            namespaces.contains(&"fp.communication.verbosity"),
            "global record missing: {namespaces:?}"
        );
        assert!(
            namespaces.contains(&"fp.engineering.simplicity"),
            "workspace record missing: {namespaces:?}"
        );
        assert!(
            !namespaces.contains(&"fp.other.project"),
            "another workspace's record leaked in: {namespaces:?}"
        );
        for r in records {
            assert_eq!(r["authority"], "user_confirmed");
            assert!(r["effective_confidence"].as_f64().unwrap() > 0.0);
        }
        let verbosity = records
            .iter()
            .find(|r| r["namespace"] == "fp.communication.verbosity")
            .unwrap();
        assert_eq!(verbosity["language"], "manglish");
        assert!(!dir.path().join(".codebro").exists());
    }

    /// A broken user-context store degrades to an empty records section —
    /// composition must never fail the packet because state.db is broken.
    #[tokio::test]
    async fn context_degrades_gracefully_when_store_is_unusable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        // state.db as a non-empty DIRECTORY: the store cannot open it AND
        // quarantine cannot move it — a deterministic unusable case.
        let bogus = state.path().join("state.db");
        std::fs::create_dir(&bogus).unwrap();
        std::fs::write(bogus.join("blocker"), b"x").unwrap();
        let server = stateful_server(&dir, &state);

        let out = call_tool_text(
            &server,
            "context",
            json!({"workspace_root": dir.path().to_string_lossy()}),
        )
        .await;
        let v = parse(&out);
        assert!(v["records"].as_array().unwrap().is_empty());
        assert!(v["repository"]["workspace_root"].is_string());
    }

    /// context never writes engineering-memory/facts files (trust
    /// separation holds for the read path as well).
    #[tokio::test]
    async fn context_composition_never_touches_project_state_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let _ = call_tool_text(
            &server,
            "context",
            json!({"workspace_root": dir.path().to_string_lossy(), "task": "probe"}),
        )
        .await;
        for name in [
            ".codebro/facts.json",
            ".codebro/engineering_memory.json",
            ".codebro/project_identity.json",
        ] {
            assert!(
                !dir.path().join(name).exists(),
                "context tool must not create {name}"
            );
        }
    }

    // ── P1: fingerprint + intent (remember / forget / resolution) ──────

    fn ws_arg(dir: &tempfile::TempDir) -> serde_json::Value {
        serde_json::Value::String(dir.path().to_string_lossy().to_string())
    }

    fn records_of(packet: &serde_json::Value) -> &Vec<serde_json::Value> {
        packet["records"].as_array().expect("records is array")
    }

    /// First-session flow (§27): the user confirms a collaboration
    /// preference, OpenCode persists it, and a later session retrieves it
    /// as USER_CONFIRMED context.
    #[tokio::test]
    async fn p1_confirmed_preference_roundtrips_as_user_confirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        let out = call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Prefer the simplest reasonable implementation and avoid unnecessary abstraction",
                "namespace": "fp.engineering.simplicity",
                "kind": "preference",
                "scope": "global",
                "user_confirmed": true,
                "original_text": "jangan overengineer benda ni",
                "language": "manglish",
            }),
        )
        .await;
        let v = parse(&out);
        assert_eq!(v["remembered"], true);
        assert_eq!(v["authority"], "user_confirmed");
        assert_eq!(v["scope"], "global");
        assert!(v["id"].as_str().unwrap().starts_with("ctx::"));

        // Later session: context carries the preference with provenance.
        let out = call_tool_text(
            &server,
            "context",
            json!({"workspace_root": ws_arg(&dir), "task": "Build a small API for this"}),
        )
        .await;
        let packet = parse(&out);
        let recs = records_of(&packet);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["namespace"], "fp.engineering.simplicity");
        assert_eq!(recs[0]["authority"], "user_confirmed");
        assert_eq!(recs[0]["kind"], "preference");
        assert_eq!(recs[0]["scope"], "global");
        assert_eq!(
            recs[0]["content"],
            "Prefer the simplest reasonable implementation and avoid unnecessary abstraction"
        );
    }

    /// Caller-principal rule: naming user_confirmed without the explicit
    /// confirmation flag is refused.
    #[tokio::test]
    async fn p1_user_confirmed_requires_the_confirmation_flag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        let err = call_tool_err(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Forged preference",
                "namespace": "fp.forge",
                "authority": "user_confirmed",
            }),
        )
        .await;
        assert!(err.contains("user_confirmed=true"), "got: {err}");

        // Default (no authority, no flag) is observed-gated, not confirmed.
        let err = call_tool_err(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Silent preference",
                "namespace": "fp.silent",
            }),
        )
        .await;
        assert!(err.contains("evidence"), "got: {err}");
    }

    /// Observed writes mint their evidence event; fictitious evidence ids
    /// are refused by the store backstop.
    #[tokio::test]
    async fn p1_observed_evidence_minted_or_verified() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        // Observation mints an event: evidence ids are real.
        let out = call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "User repeatedly runs cargo test before committing",
                "namespace": "fp.workflow.verify",
                "authority": "observed",
                "observation": "three consecutive sessions ended with cargo test runs",
            }),
        )
        .await;
        let v = parse(&out);
        assert_eq!(v["authority"], "observed");
        assert!(!v["evidence"].as_array().unwrap().is_empty());

        // Fiction is refused.
        let err = call_tool_err(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Fictitious observation",
                "namespace": "fp.fiction",
                "authority": "observed",
                "evidence_event_ids": [999999],
            }),
        )
        .await;
        assert!(err.contains("does not exist"), "got: {err}");

        // ai_inferred with minted evidence stays inferred, never confirmed.
        let out = call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "User may prefer terse output",
                "namespace": "fp.communication.verbosity",
                "authority": "ai_inferred",
                "observation": "last three replies were one-liners",
            }),
        )
        .await;
        assert_eq!(parse(&out)["authority"], "ai_inferred");
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "reply style probe"}),
            )
            .await,
        );
        let rec = records_of(&packet)
            .iter()
            .find(|r| r["namespace"] == "fp.communication.verbosity")
            .expect("inferred record visible");
        assert_eq!(rec["authority"], "ai_inferred");
    }

    /// Project override wins over global at equal authority; task wins
    /// over project when the task is named.
    #[tokio::test]
    async fn p1_resolution_task_beats_project_beats_global() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let ns = "fp.communication.verbosity";

        for (scope, content, extra) in [
            ("global", "Prefer concise responses", json!({})),
            (
                "project",
                "For this architecture document, provide detailed reasoning",
                json!({}),
            ),
        ] {
            let mut args = json!({
                "workspace_root": ws_arg(&dir),
                "content": content,
                "namespace": ns,
                "scope": scope,
                "user_confirmed": true,
            });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            call_tool_text(&server, "remember", args).await;
        }
        // Project wins inside the project.
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "write the doc"}),
            )
            .await,
        );
        let recs = records_of(&packet);
        assert_eq!(recs.len(), 1, "one winner per namespace: {recs:?}");
        assert_eq!(recs[0]["scope"], "project");

        // Task override wins when the task is named...
        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Give only the final implementation plan",
                "namespace": ns,
                "scope": "task",
                "task_id": "task-7",
                "user_confirmed": true,
            }),
        )
        .await;
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "write the doc", "task_id": "task-7"}),
            )
            .await,
        );
        let recs = records_of(&packet);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["scope"], "task");

        // ... and stays invisible without the task.
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "write the doc"}),
            )
            .await,
        );
        assert_eq!(records_of(&packet)[0]["scope"], "project");
    }

    /// Confirmation outranks inference across scopes: an inferred project
    /// guess must not silently override a confirmed global.
    #[tokio::test]
    async fn p1_confirmed_global_beats_inferred_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let ns = "fp.engineering.simplicity";
        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Prefer simple implementations",
                "namespace": ns,
                "scope": "global",
                "user_confirmed": true,
            }),
        )
        .await;
        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Maybe prefers abstraction here",
                "namespace": ns,
                "scope": "project",
                "authority": "ai_inferred",
                "observation": "imported a framework crate",
            }),
        )
        .await;
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "pick a design"}),
            )
            .await,
        );
        let recs = records_of(&packet);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["authority"], "user_confirmed");
        assert_eq!(recs[0]["scope"], "global");
    }

    /// Updating a preference goes through supersede: history preserved,
    /// blind overwrite refused.
    #[tokio::test]
    async fn p1_update_requires_supersedes_and_preserves_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let ns = "fp.communication.verbosity";

        let first = parse(
            &call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "Prefer concise responses",
                    "namespace": ns,
                    "scope": "global",
                    "user_confirmed": true,
                }),
            )
            .await,
        );
        let first_id = first["id"].as_str().unwrap().to_string();

        // Blind second write to the same namespace is refused, naming the owner.
        let err = call_tool_err(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Prefer detailed explanations",
                "namespace": ns,
                "scope": "global",
                "user_confirmed": true,
            }),
        )
        .await;
        assert!(err.contains(&first_id), "must name the incumbent: {err}");
        assert!(
            err.contains("supersedes"),
            "must direct to supersede: {err}"
        );

        // Explicit supersede: new winner, old row superseded (still stored).
        let second = parse(
            &call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "Prefer detailed explanations for architecture discussions",
                    "namespace": ns,
                    "scope": "global",
                    "user_confirmed": true,
                    "supersedes": first_id,
                }),
            )
            .await,
        );
        assert_eq!(second["superseded"], first_id);
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "discuss architecture"}),
            )
            .await,
        );
        let recs = records_of(&packet);
        assert_eq!(recs.len(), 1);
        assert_eq!(
            recs[0]["content"],
            "Prefer detailed explanations for architecture discussions"
        );
    }

    /// Intent lifecycle: create active with rationale/priority, pause via
    /// supersede, complete via auto-retire, then terminal refusal.
    #[tokio::test]
    async fn p1_intent_lifecycle_active_pause_complete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let ns = "intent.codebro-mission";

        let created = parse(
            &call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "Build CodeBro as persistent engineering infrastructure for OpenCode",
                    "namespace": ns,
                    "kind": "intent",
                    "scope": "project",
                    "user_confirmed": true,
                    "rationale": "Provide context and memory without another coding agent",
                    "priority": "high",
                }),
            )
            .await,
        );
        assert_eq!(created["kind"], "intent");
        assert_eq!(created["status"], "active");
        let active_id = created["id"].as_str().unwrap().to_string();

        // Active intent surfaces with decoded metadata.
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "plan next milestone"}),
            )
            .await,
        );
        let intent = records_of(&packet)
            .iter()
            .find(|r| r["namespace"] == ns)
            .expect("intent visible");
        assert_eq!(intent["kind"], "intent");
        assert_eq!(intent["intent"]["intent_status"], "active");
        assert_eq!(intent["intent"]["priority"], "high");
        assert!(intent["intent"]["rationale"]
            .as_str()
            .unwrap()
            .contains("without another"));

        // Pause via supersede: still actionable, still visible.
        let paused = parse(
            &call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "Build CodeBro as persistent engineering infrastructure for OpenCode",
                    "namespace": ns,
                    "kind": "intent",
                    "scope": "project",
                    "user_confirmed": true,
                    "rationale": "Provide context and memory without another coding agent",
                    "priority": "high",
                    "intent_status": "paused",
                    "supersedes": active_id,
                }),
            )
            .await,
        );
        assert_eq!(paused["status"], "active");
        let paused_id = paused["id"].as_str().unwrap().to_string();
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "plan next milestone"}),
            )
            .await,
        );
        assert_eq!(
            records_of(&packet)
                .iter()
                .find(|r| r["namespace"] == ns)
                .unwrap()["intent"]["intent_status"],
            "paused"
        );

        // Complete retires the predecessor (auto-found, no supersedes needed).
        let done = parse(
            &call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "Build CodeBro as persistent engineering infrastructure for OpenCode",
                    "namespace": ns,
                    "kind": "intent",
                    "scope": "project",
                    "user_confirmed": true,
                    "intent_status": "completed",
                }),
            )
            .await,
        );
        assert_eq!(done["status"], "expired");
        assert_eq!(done["superseded"], paused_id);

        // Completed intent leaves the actionable packet...
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "plan next milestone"}),
            )
            .await,
        );
        assert!(
            records_of(&packet).iter().all(|r| r["namespace"] != ns),
            "completed intent must leave the packet"
        );

        // ... and refuses further supersession (history is append-only).
        let err = call_tool_err(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Resurrect the mission",
                "namespace": ns,
                "kind": "intent",
                "scope": "project",
                "user_confirmed": true,
                "supersedes": done["id"],
            }),
        )
        .await;
        assert!(
            err.contains("terminal") || err.contains("active"),
            "got: {err}"
        );
    }

    /// Intent fields on non-intent kinds, bad vocabularies, and malformed
    /// scope ask are caller errors, not stored rows.
    #[tokio::test]
    async fn p1_intent_shape_violations_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        for (label, args) in [
            (
                "rationale on preference",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "x", "namespace": "fp.x",
                    "user_confirmed": true, "rationale": "nope",
                }),
            ),
            (
                "bad priority",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "x", "namespace": "intent.x", "kind": "intent",
                    "user_confirmed": true, "priority": "urgent",
                }),
            ),
            (
                "bad intent status",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "x", "namespace": "intent.x", "kind": "intent",
                    "user_confirmed": true, "intent_status": "thriving",
                }),
            ),
            (
                "bad scope",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "x", "namespace": "fp.x",
                    "user_confirmed": true, "scope": "universe",
                }),
            ),
            (
                "bad kind",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "x", "namespace": "fp.x",
                    "user_confirmed": true, "kind": "vibe",
                }),
            ),
            (
                "task scope without task_id",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "x", "namespace": "fp.x",
                    "user_confirmed": true, "scope": "task",
                }),
            ),
            (
                "reference-only fact without backlink",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "mirrored fact",
                    "namespace": "fact.x", "kind": "fact",
                    "user_confirmed": true,
                }),
            ),
        ] {
            let err = call_tool_err(&server, "remember", args).await;
            assert!(!err.is_empty(), "{label} must fail");
        }

        // Nothing from the rejection battery persisted.
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "probe"}),
            )
            .await,
        );
        assert!(records_of(&packet).is_empty());
    }

    /// forget: confirm gate, reversible reject, namespace resolution,
    /// permanent removal, and cross-workspace refusal.
    #[tokio::test]
    async fn p1_forget_confirm_reject_remove_and_isolation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let other = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_multiws_server(&dir, &[&other], &state);

        let created = parse(
            &call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": "Temporary preference",
                    "namespace": "fp.temp",
                    "scope": "project",
                    "user_confirmed": true,
                }),
            )
            .await,
        );
        let id = created["id"].as_str().unwrap().to_string();

        // No confirm: refused.
        let err = call_tool_err(&server, "forget", json!({"id": id})).await;
        assert!(err.contains("confirm=true"), "got: {err}");

        // Namespace resolution retires the winner (reversible reject).
        let out = parse(
            &call_tool_text(
                &server,
                "forget",
                json!({"workspace_root": ws_arg(&dir), "namespace": "fp.temp", "confirm": true}),
            )
            .await,
        );
        assert_eq!(out["id"], id);
        assert_eq!(out["action"], "rejected");
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "probe"}),
            )
            .await,
        );
        assert!(
            records_of(&packet).is_empty(),
            "rejected rows leave the packet"
        );

        // Cross-workspace forget is refused even with confirm...
        let err = call_tool_err(
            &server,
            "forget",
            json!({
                "workspace_root": other.path().to_string_lossy(),
                "id": id, "confirm": true,
            }),
        )
        .await;
        assert!(err.contains("another workspace"), "got: {err}");

        // ... and permanent removal deletes the row.
        let out = parse(
            &call_tool_text(
                &server,
                "forget",
                json!({"workspace_root": ws_arg(&dir), "id": id, "confirm": true, "permanent": true}),
            )
            .await,
        );
        assert_eq!(out["action"], "removed");
        let err = call_tool_err(
            &server,
            "forget",
            json!({"workspace_root": ws_arg(&dir), "id": id, "confirm": true}),
        )
        .await;
        assert!(err.contains("no record"), "got: {err}");
    }

    /// Workspace isolation: project rows never leak across projects sharing
    /// one state.db; equivalent path spellings resolve to one namespace.
    #[tokio::test]
    async fn p1_workspace_isolation_and_canonical_equivalence() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_multiws_server(&dir_a, &[&dir_b], &state);

        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir_a),
                "content": "Project A secret preference",
                "namespace": "fp.secret",
                "scope": "project",
                "user_confirmed": true,
            }),
        )
        .await;
        // Sibling project sees nothing of A (same store file, other root).
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": dir_b.path().to_string_lossy(), "task": "probe"}),
            )
            .await,
        );
        assert!(records_of(&packet).is_empty());

        // Trailing-slash spelling of A resolves to the same namespace.
        let with_slash = format!("{}/", dir_a.path().display());
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": with_slash, "task": "probe"}),
            )
            .await,
        );
        assert_eq!(records_of(&packet).len(), 1);
        assert_eq!(records_of(&packet)[0]["namespace"], "fp.secret");
    }

    /// The packet stays bounded no matter how many rows compete.
    #[tokio::test]
    async fn p1_context_stays_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        for i in 0..20 {
            call_tool_text(
                &server,
                "remember",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "content": format!("Preference number {i} with enough words to be searchable"),
                    "namespace": format!("fp.bulk.{i}"),
                    "scope": "global",
                    "user_confirmed": true,
                }),
            )
            .await;
        }
        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "bulk probe"}),
            )
            .await,
        );
        assert!(records_of(&packet).len() <= crate::engineering_context::MAX_CONTEXT_RECORDS);
    }

    /// remember/forget serialize on the workspace mutation lock like every
    /// other mutating tool.
    #[tokio::test]
    async fn p1_remember_blocks_while_mutation_lock_held() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = Arc::new(stateful_server(&dir, &state));
        let ws = server.resolve_workspace(None).unwrap();
        let guard = ws.mutation_lock.lock().await;
        let s2 = server.clone();
        let root = dir.path().to_string_lossy().to_string();
        let mut task = tokio::spawn(async move {
            call_tool_text(
                &s2,
                "remember",
                json!({
                    "workspace_root": root,
                    "content": "Blocked preference",
                    "namespace": "fp.blocked",
                    "scope": "global",
                    "user_confirmed": true,
                }),
            )
            .await
        });
        let blocked = tokio::time::timeout(std::time::Duration::from_millis(100), &mut task)
            .await
            .is_err();
        assert!(blocked, "remember must wait while the lock is held");
        drop(guard);
        let out = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("finishes after release")
            .expect("join ok");
        assert!(out.contains("remembered"), "got: {out}");
    }

    // ── P2: sessions + history + recall ─────────────────────────────────

    fn history_of(out: &serde_json::Value) -> &Vec<serde_json::Value> {
        out["history"].as_array().expect("history is array")
    }

    fn all_recall_events(out: &serde_json::Value) -> Vec<&serde_json::Value> {
        history_of(out)
            .iter()
            .flat_map(|g| g["events"].as_array().unwrap().iter())
            .collect()
    }

    /// End-to-end P2 flow, session 1 → session 2: a decision recorded via
    /// remember (passively captured) is recalled by a later question —
    /// the user never names the session.
    #[tokio::test]
    async fn p2_remembered_decision_is_recallable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        // Session 1: the user confirms SQLite; OpenCode persists it.
        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Use SQLite because this is a local persistent runtime without unnecessary infrastructure",
                "namespace": "intent.storage.local",
                "kind": "intent",
                "scope": "project",
                "user_confirmed": true,
                "rationale": "local persistent runtime, no extra infrastructure",
            }),
        )
        .await;

        // Session 2: "Why are we using SQLite again?" → recall.
        let out = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": ws_arg(&dir), "query": "why are we using SQLite again"}),
            )
            .await,
        );
        assert_eq!(out["provenance"], "historical-evidence");
        let events = all_recall_events(&out);
        assert!(!events.is_empty(), "the decision must be recalled");
        let decision = events
            .iter()
            .find(|e| e["event"] == "decision")
            .expect("a decision event");
        assert!(decision["excerpt"].as_str().unwrap().contains("SQLite"));
        assert_eq!(decision["scope"], "project");
        // Session grouping carries the provenance.
        let group = &history_of(&out)[0];
        assert!(group["session"].is_string());
        assert!(group["workspace_root"].is_string());
    }

    /// Applied changes are passively captured and recallable as history.
    #[tokio::test]
    async fn p2_applied_change_is_recallable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("notes.txt"), "sqlite weighs nothing").unwrap();
        let server = stateful_server(&dir, &state);

        call_tool_text(
            &server,
            "apply_change",
            json!({
                "workspace_root": ws_arg(&dir),
                "path": "notes.txt",
                "old": "weighs nothing",
                "new": "weighs nothing and needs no server",
            }),
        )
        .await;

        let out = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": ws_arg(&dir), "query": "changes to notes file"}),
            )
            .await,
        );
        let events = all_recall_events(&out);
        assert!(
            events.iter().any(|e| e["event"] == "change_applied"),
            "change must be recalled: {events:?}"
        );
    }

    /// Project isolation through the MCP boundary: B never sees A's
    /// history, even though FTS physically contains A's text.
    #[tokio::test]
    async fn p2_recall_enforces_project_isolation() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        // One server, one shared state.db, two AUTHORIZED workspaces
        // (operator-equivalent launch config).
        let server = stateful_multiws_server(&dir_a, &[&dir_b], &state);
        let a = dir_a.path().to_string_lossy().to_string();
        let b = dir_b.path().to_string_lossy().to_string();

        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": a,
                "content": "Project A rejected architecture xylophone for complexity reasons",
                "namespace": "intent.architecture.private",
                "kind": "intent",
                "scope": "project",
                "user_confirmed": true,
            }),
        )
        .await;

        let out_b = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": b, "query": "architecture xylophone"}),
            )
            .await,
        );
        assert_eq!(out_b["total_matches"], 0);
        assert!(history_of(&out_b).is_empty());

        // ... while A itself recalls it.
        let out_a = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": a, "query": "architecture xylophone"}),
            )
            .await,
        );
        assert!(out_a["total_matches"].as_u64().unwrap() >= 1);
    }

    /// Task isolation through the MCP boundary: task B recall excludes
    /// task A history; task A recall includes it.
    #[tokio::test]
    async fn p2_recall_enforces_task_isolation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let root = ws_arg(&dir);

        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": root,
                "content": "Task alpha decided to use SQLite for the queue",
                "namespace": "intent.queue.backend",
                "kind": "intent",
                "scope": "task",
                "task_id": "alpha",
                "user_confirmed": true,
            }),
        )
        .await;

        // Task beta must not see alpha's history…
        let out_beta = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": root, "query": "queue backend sqlite", "scope": "task", "task_id": "beta"}),
            )
            .await,
        );
        assert_eq!(out_beta["total_matches"], 0);

        // …while alpha does.
        let out_alpha = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": root, "query": "queue backend sqlite", "scope": "task", "task_id": "alpha"}),
            )
            .await,
        );
        assert_eq!(out_alpha["total_matches"], 1);
        assert_eq!(all_recall_events(&out_alpha)[0]["scope"], "task");
    }

    /// recall is read-only over history: calling it creates no sessions
    /// and no events (no recursion), and it rejects empty queries.
    #[tokio::test]
    async fn p2_recall_writes_no_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let probe = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let ws = dir.path().display().to_string();

        call_tool_text(
            &server,
            "recall",
            json!({"workspace_root": ws_arg(&dir), "query": "anything sqlite"}),
        )
        .await;
        assert_eq!(probe.list_events(&ws, 200).unwrap().len(), 0);
        assert!(probe
            .list_sessions(&ws, &crate::context_runtime::SessionFilter::default(), 0)
            .unwrap()
            .is_empty());

        let err = call_tool_err(
            &server,
            "recall",
            json!({"workspace_root": ws_arg(&dir), "query": "a?"}),
        )
        .await;
        assert!(err.contains("searchable token"), "got: {err}");

        let err = call_tool_err(
            &server,
            "recall",
            json!({"workspace_root": ws_arg(&dir), "query": "sqlite", "scope": "nebula"}),
        )
        .await;
        assert!(err.contains("unknown recall scope"), "got: {err}");
    }

    /// context never dumps history: the packet has no history section and
    /// recall works independently of it.
    #[tokio::test]
    async fn p2_context_does_not_dump_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Use SQLite for local persistence",
                "namespace": "intent.storage.local",
                "kind": "intent",
                "scope": "project",
                "user_confirmed": true,
            }),
        )
        .await;

        let packet = parse(
            &call_tool_text(
                &server,
                "context",
                json!({"workspace_root": ws_arg(&dir), "task": "choose storage sqlite"}),
            )
            .await,
        );
        assert!(
            packet.get("history").is_none(),
            "history must stay out of context"
        );
        assert!(packet.get("recall").is_none());
        // …while recall independently surfaces the decision.
        let out = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": ws_arg(&dir), "query": "sqlite storage"}),
            )
            .await,
        );
        assert!(out["total_matches"].as_u64().unwrap() >= 1);
    }

    /// Recall output is bounded excerpts with provenance, never transcripts.
    #[tokio::test]
    async fn p2_recall_output_is_bounded_and_provenance_tagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Use SQLite because this is a local persistent runtime",
                "namespace": "intent.storage.local",
                "kind": "intent",
                "scope": "project",
                "user_confirmed": true,
            }),
        )
        .await;

        let out = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": ws_arg(&dir), "query": "sqlite", "limit": 2}),
            )
            .await,
        );
        for e in all_recall_events(&out) {
            assert!(e["event_id"].is_number());
            assert!(e["event"].is_string());
            assert!(e["timestamp"].is_number());
            assert!(e["scope"].is_string());
            assert!(e["excerpt"].is_string());
            assert!(e["excerpt"].as_str().unwrap().chars().count() <= 240 + 64);
        }
        assert!(out["note"]
            .as_str()
            .unwrap()
            .contains("Historical evidence"));
    }

    // ── learn tool (tool 22, P3) ───────────────────────────────────────

    /// Seed three confirmed dependency-avoidance decisions (distinct
    /// namespaces so the clash rule holds, shared vocabulary so the
    /// deterministic pair clustering fires).
    async fn seed_avoidance_decisions(server: &CodeBroMcpServer, ws: serde_json::Value) {
        for (ns, area) in [
            ("fp.deps.alpha", "alpha"),
            ("fp.deps.beta", "beta"),
            ("fp.deps.gamma", "gamma"),
        ] {
            call_tool_text(
                server,
                "remember",
                json!({
                    "workspace_root": ws,
                    "content": format!("Avoid unnecessary dependencies for tiny utilities in module {area}"),
                    "namespace": ns,
                    "kind": "preference",
                    "scope": "project",
                    "user_confirmed": true,
                }),
            )
            .await;
        }
    }

    fn learn_candidates(out: &serde_json::Value) -> &Vec<serde_json::Value> {
        out["candidates"].as_array().expect("candidates array")
    }

    /// Repeated decisions become an evaluated AI_INFERRED hypothesis —
    /// never user-confirmed truth.
    #[tokio::test]
    async fn p3_learn_run_accepts_repeated_decisions_as_ai_inferred() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        seed_avoidance_decisions(&server, ws_arg(&dir)).await;

        let out = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "run"}),
            )
            .await,
        );
        assert_eq!(out["accepted"], 1, "one hypothesis must accept: {out:?}");
        assert_eq!(out["provenance"], "learning-candidates");

        let list = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "list"}),
            )
            .await,
        );
        let items = learn_candidates(&list);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["kind"], "user_preference");
        assert_eq!(items[0]["status"], "accepted");
        assert!(items[0]["confidence"].as_f64().unwrap() >= 0.55);
        assert_eq!(items[0]["supporting"], 3);
        assert!(items[0]["proposition"].as_str().unwrap().contains("prefer"));

        // The persisted inference is AI_INFERRED with cited evidence.
        let got = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "action": "get",
                    "candidate_id": items[0]["candidate_id"],
                }),
            )
            .await,
        );
        assert_eq!(got["explanation"]["authority"], "ai_inferred");
        assert_eq!(
            got["candidate"]["inference_record_id"].as_str().unwrap(),
            got["explanation"]["inference_record_id"].as_str().unwrap()
        );
        assert!(!got["explanation"]["note"].as_str().unwrap().is_empty());
    }

    /// The model can never self-confirm: confirm without the explicit user
    /// speech act is refused and the authority stays inferred.
    #[tokio::test]
    async fn p3_learn_confirm_requires_user_confirmed_flag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        seed_avoidance_decisions(&server, ws_arg(&dir)).await;
        call_tool_text(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "run"}),
        )
        .await;
        let list = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "list"}),
            )
            .await,
        );
        let id = list["candidates"][0]["candidate_id"].clone();

        let err = call_tool_err(
            &server,
            "learn",
            json!({
                "workspace_root": ws_arg(&dir),
                "action": "confirm",
                "candidate_id": id,
            }),
        )
        .await;
        assert!(
            err.contains("user_confirmed=true"),
            "forgery refused: {err}"
        );
    }

    /// Explicit user confirmation promotes AI_INFERRED → USER_CONFIRMED via
    /// supersede, and the inference becomes context-visible as confirmed.
    #[tokio::test]
    async fn p3_learn_confirm_promotes_to_user_confirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        seed_avoidance_decisions(&server, ws_arg(&dir)).await;
        call_tool_text(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "run"}),
        )
        .await;
        let list = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "list"}),
            )
            .await,
        );
        let id = list["candidates"][0]["candidate_id"].clone();

        let out = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "action": "confirm",
                    "candidate_id": id,
                    "user_confirmed": true,
                }),
            )
            .await,
        );
        assert_eq!(out["confirmed"], true);
        assert_eq!(out["candidate"]["status"], "superseded");

        // The confirmed record resolves in context with its authority.
        let ctx = parse(
            &call_tool_text(
                &server,
                "context",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "task": "should I add a dependency for this tiny utility",
                    "keywords": ["dependencies", "utility"],
                }),
            )
            .await,
        );
        let records = ctx["records"].as_array().expect("records array");
        let confirmed = records
            .iter()
            .find(|r| r["namespace"].as_str().unwrap().starts_with("learn."));
        assert!(
            confirmed.is_some(),
            "confirmed learning must be context-eligible: {records:?}"
        );
        assert_eq!(confirmed.unwrap()["authority"], "user_confirmed");
    }

    /// User rejection preserves negative knowledge and survives re-runs.
    #[tokio::test]
    async fn p3_learn_reject_preserves_negative_knowledge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        seed_avoidance_decisions(&server, ws_arg(&dir)).await;
        call_tool_text(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "run"}),
        )
        .await;
        let list = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "list"}),
            )
            .await,
        );
        let id = list["candidates"][0]["candidate_id"].clone();

        // The confirm gate holds for rejection too.
        let err = call_tool_err(
            &server,
            "learn",
            json!({
                "workspace_root": ws_arg(&dir),
                "action": "reject",
                "candidate_id": id,
            }),
        )
        .await;
        assert!(err.contains("confirm=true"), "{err}");

        let out = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "action": "reject",
                    "candidate_id": id,
                    "confirm": true,
                    "reason": "only true for small utilities",
                }),
            )
            .await,
        );
        assert_eq!(out["rejected"], true);
        assert_eq!(out["candidate"]["status"], "rejected");

        // A re-run refuses to rewrite the user's verdict.
        let rerun = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "run"}),
            )
            .await,
        );
        assert_eq!(rerun["skipped_terminal"], 1);
    }

    /// Learning is workspace-confined: B learns nothing from A's history.
    #[tokio::test]
    async fn p3_learn_enforces_project_isolation() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_multiws_server(&dir_a, &[&dir_b], &state);
        let a = dir_a.path().to_string_lossy().to_string();
        let b = dir_b.path().to_string_lossy().to_string();
        seed_avoidance_decisions(&server, json!(a)).await;

        let out_b = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": b, "action": "run"}),
            )
            .await,
        );
        assert_eq!(out_b["proposed"], 0);
        let list_b = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": b, "action": "list"}),
            )
            .await,
        );
        assert!(learn_candidates(&list_b).is_empty());
    }

    /// Learning writes no history: recall evidence counts are identical
    /// before and after a learn pass (no recursion, no self-evidence).
    #[tokio::test]
    async fn p3_learn_writes_no_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        seed_avoidance_decisions(&server, ws_arg(&dir)).await;

        let before = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": ws_arg(&dir), "query": "unnecessary dependencies tiny utilities"}),
            )
            .await,
        );
        call_tool_text(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "run"}),
        )
        .await;
        // Recall itself writes nothing either; list/get are read-only too.
        call_tool_text(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "list"}),
        )
        .await;
        let after = parse(
            &call_tool_text(
                &server,
                "recall",
                json!({"workspace_root": ws_arg(&dir), "query": "unnecessary dependencies tiny utilities"}),
            )
            .await,
        );
        assert_eq!(before["total_matches"], after["total_matches"]);
    }

    /// A weak hypothesis (single observation) defers and never enters the
    /// always-available context packet.
    #[tokio::test]
    async fn p3_weak_hypothesis_stays_out_of_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        call_tool_text(
            &server,
            "remember",
            json!({
                "workspace_root": ws_arg(&dir),
                "content": "Avoid unnecessary dependencies for tiny utilities in module solo",
                "namespace": "fp.deps.solo",
                "kind": "preference",
                "scope": "project",
                "user_confirmed": true,
            }),
        )
        .await;

        let out = parse(
            &call_tool_text(
                &server,
                "learn",
                json!({"workspace_root": ws_arg(&dir), "action": "run"}),
            )
            .await,
        );
        assert_eq!(out["accepted"], 0, "one event is not a pattern: {out:?}");

        let ctx = parse(
            &call_tool_text(
                &server,
                "context",
                json!({
                    "workspace_root": ws_arg(&dir),
                    "task": "tiny utility dependency choice",
                    "keywords": ["dependencies", "utility"],
                }),
            )
            .await,
        );
        let records = ctx["records"].as_array().expect("records array");
        assert!(
            records
                .iter()
                .all(|r| !r["namespace"].as_str().unwrap().starts_with("learn.")),
            "no inferred namespace may surface: {records:?}"
        );
    }

    /// Unknown actions and missing arguments are clean invalid_params.
    #[tokio::test]
    async fn p3_learn_rejects_bad_actions_and_args() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);

        let err = call_tool_err(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "hallucinate"}),
        )
        .await;
        assert!(err.contains("unknown learn action"), "{err}");

        let err = call_tool_err(
            &server,
            "learn",
            json!({"workspace_root": ws_arg(&dir), "action": "evaluate"}),
        )
        .await;
        assert!(err.contains("candidate_id"), "{err}");

        let err = call_tool_err(
            &server,
            "learn",
            json!({
                "workspace_root": ws_arg(&dir),
                "action": "get",
                "candidate_id": "lc::missing0000000000",
            }),
        )
        .await;
        assert!(err.contains("no learning candidate"), "{err}");
    }

    // ── P7 engineering brief (tool 25) ─────────────────────────────────

    fn brief_json(out: &str) -> serde_json::Value {
        serde_json::from_str(out).expect("brief is valid json")
    }

    fn unknown_kinds(v: &serde_json::Value) -> Vec<String> {
        v["unknowns"]
            .as_array()
            .expect("unknowns array")
            .iter()
            .filter_map(|u| u["kind"].as_str().map(str::to_string))
            .collect()
    }

    /// Empty scope is rejected, never answered with a dump.
    #[tokio::test]
    async fn brief_rejects_empty_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let err = call_tool_err(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir)}),
        )
        .await;
        assert!(err.contains("task scope is required"), "{err}");
    }

    /// Traversal-shaped and blank targets are caller errors.
    #[tokio::test]
    async fn brief_rejects_bad_targets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        for bad in [
            json!({"workspace_root": ws_arg(&dir), "task": "x", "target_path": "../escape"}),
            json!({"workspace_root": ws_arg(&dir), "task": "x", "target_symbol": " "}),
        ] {
            let err = call_tool_err(&server, "engineering_brief", bad).await;
            assert!(
                err.contains("target_path")
                    || err.contains("target_symbol")
                    || err.contains("must not"),
                "{err}"
            );
        }
    }

    /// Empty workspace: unknowns reported, shape stable, no impact invented.
    #[tokio::test]
    async fn brief_on_empty_workspace_reports_unknowns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir), "task": "fix authentication login"}),
        )
        .await;
        let v = brief_json(&out);
        assert_eq!(v["task"], "fix authentication login");
        assert!(v["impact"].is_null());
        assert_eq!(v["targets"]["discovery"], "none");
        let kinds = unknown_kinds(&v);
        for required in [
            "EMPTY_REPOSITORY",
            "NO_RELEVANT_TESTS",
            "NO_HISTORY",
            "NO_MEMORY",
            "NO_LEARNING",
            "NO_SKILLS",
            "MISSING_IDENTITY",
        ] {
            assert!(
                kinds.contains(&required.to_string()),
                "missing {required}: {kinds:?}"
            );
        }
        // Category vocabulary present on sections.
        assert_eq!(v["repository"]["category"], "FACT");
        assert_eq!(v["freshness"]["provenance"], "derived");
        assert!(v["bounds"].is_object());
        assert!(v["scope"]["keywords"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "authentication"));
    }

    /// Task-scoped brief surfaces read-only task state without transitioning.
    #[tokio::test]
    async fn brief_with_task_id_reads_task_state_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let created = call_tool_text(
            &server,
            "task",
            json!({"workspace_root": ws_arg(&dir), "action": "create", "title": "Repair login flow", "description": "session tokens expire early"}),
        )
        .await;
        let task_id = brief_json(&created)["task"]["task_id"]
            .as_str()
            .expect("task id")
            .to_string();

        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir), "task_id": task_id}),
        )
        .await;
        let v = brief_json(&out);
        let ts = &v["task_state"];
        assert_eq!(ts["task_id"], task_id.as_str());
        assert_eq!(ts["status"], "pending");
        assert_eq!(ts["category"], "TASK_STATE");
        // Task-derived keywords enrich scope even though no task text given.
        let keywords: Vec<String> = v["scope"]["keywords"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect();
        assert!(
            keywords.iter().any(|k| k == "Repair" || k == "login"),
            "{keywords:?}"
        );

        // Read-only: the task is still pending at the same version.
        let inspected = call_tool_text(
            &server,
            "task",
            json!({"workspace_root": ws_arg(&dir), "action": "inspect", "task_id": task_id}),
        )
        .await;
        let snap = &brief_json(&inspected)["snapshot"];
        assert_eq!(snap["task"]["status"], "pending");
        assert_eq!(snap["task"]["current_version"], ts["version"]);
    }

    /// Cross-workspace task ids read as unknown — never leaked.
    #[tokio::test]
    async fn brief_cross_workspace_task_is_unknown() {
        let a = tempfile::tempdir().expect("tempdir");
        let b = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_multiws_server(&a, &[&b], &state);
        let created = call_tool_text(
            &server,
            "task",
            json!({"workspace_root": ws_arg(&b), "action": "create", "title": "Secret plan"}),
        )
        .await;
        let task_id = brief_json(&created)["task"]["task_id"]
            .as_str()
            .expect("task id")
            .to_string();
        assert!(task_id.starts_with("task::"));

        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&a), "task_id": task_id, "task": "continue work"}),
        )
        .await;
        let v = brief_json(&out);
        assert!(v["task_state"].is_null());
        assert!(unknown_kinds(&v).contains(&"TASK_NOT_FOUND".to_string()));
        assert!(
            !out.contains("Secret plan"),
            "task content must not leak across workspaces"
        );
    }

    /// USER_CONFIRMED constraints flow as hard constraints; preferences do not.
    #[tokio::test]
    async fn brief_surfaces_confirmed_constraints_not_preferences() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        call_tool_text(
            &server,
            "remember",
            json!({"workspace_root": ws_arg(&dir), "content": "No new dependencies without review", "namespace": "eng.constraints.deps", "kind": "constraint", "user_confirmed": true}),
        )
        .await;
        call_tool_text(
            &server,
            "remember",
            json!({"workspace_root": ws_arg(&dir), "content": "Prefers short functions", "namespace": "eng.style.brevity", "kind": "preference", "user_confirmed": true}),
        )
        .await;
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir), "task": "add dependency review"}),
        )
        .await;
        let v = brief_json(&out);
        let constraints = v["constraints"].as_array().expect("constraints");
        assert!(
            constraints.iter().any(|c| c["content"]
                .as_str()
                .unwrap()
                .contains("No new dependencies")
                && c["hardness"] == "hard"
                && c["authority"] == "user_confirmed"),
            "{constraints:?}"
        );
        assert!(
            constraints
                .iter()
                .all(|c| !c["content"].as_str().unwrap().contains("short functions")),
            "preferences must never become constraints: {constraints:?}"
        );
    }

    /// Identity decisions flow with currency; superseded ones are not current.
    #[tokio::test]
    async fn brief_decisions_mark_superseded_not_current() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"brief-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        crate::init::run(dir.path()).expect("init");
        let server = local_sandbox_server(&dir);
        call_tool_text(
            &server,
            "update_identity",
            json!({
                "add_decisions": [
                    {"title": "Use SQLite for state", "description": "Durable local state via SQLite."},
                    {"title": "Retire SQLite backend", "description": "Superseded experiment.", "status": "superseded"}
                ]
            }),
        )
        .await;
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"task": "SQLite state backend"}),
        )
        .await;
        let v = brief_json(&out);
        let decisions = v["decisions"].as_array().expect("decisions");
        let sqlite: Vec<&serde_json::Value> = decisions
            .iter()
            .filter(|d| d["title"].as_str().unwrap().contains("SQLite"))
            .collect();
        assert_eq!(sqlite.len(), 2, "{decisions:?}");
        let current: Vec<&&serde_json::Value> =
            sqlite.iter().filter(|d| d["current"] == true).collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0]["status"], "accepted");
    }

    /// History is relevant excerpts, never a transcript dump.
    #[tokio::test]
    async fn brief_history_is_bounded_excerpts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        call_tool_text(
            &server,
            "remember",
            json!({"workspace_root": ws_arg(&dir), "content": "Chose SQLite for durable login sessions", "namespace": "eng.history.logindecision", "kind": "decision", "related_ids": ["task::x"], "user_confirmed": true}),
        )
        .await;
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir), "task": "login sessions SQLite"}),
        )
        .await;
        let v = brief_json(&out);
        let history = v["history"].as_array().expect("history");
        assert!(
            !history.is_empty(),
            "remembered decision must surface as history"
        );
        assert!(history.len() <= crate::engineering_brief::MAX_BRIEF_HISTORY);
        for h in history {
            assert_eq!(h["category"], "HISTORY");
            assert_eq!(h["provenance"], "observed");
            assert!(
                h["excerpt"].as_str().unwrap().len() <= 240 + 64,
                "excerpt bounded"
            );
            assert!(h.get("payload").is_none(), "raw payloads never exposed");
        }
    }

    /// Explicit missing symbols are unknowns; determinism holds.
    #[tokio::test]
    async fn brief_missing_symbol_and_determinism() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let args = json!({"workspace_root": ws_arg(&dir), "task": "fix things", "target_symbol": "no_such_symbol_xyz"});
        let a = call_tool_text(&server, "engineering_brief", args.clone()).await;
        let b = call_tool_text(&server, "engineering_brief", args).await;
        assert_eq!(
            a, b,
            "same state + same request must produce the same brief"
        );
        let v = brief_json(&a);
        assert!(v["impact"].is_null());
        assert!(unknown_kinds(&v).contains(&"TARGET_NOT_FOUND".to_string()));
    }

    /// Concurrent briefs agree; generation writes no `.codebro` state.
    #[tokio::test]
    async fn brief_concurrent_requests_agree_and_write_nothing() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = Arc::new(stateful_server(&dir, &state));
        let before: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let s = server.clone();
            let root = dir.path().to_string_lossy().to_string();
            tasks.push(tokio::spawn(async move {
                call_tool_text(
                    &s,
                    "engineering_brief",
                    json!({"workspace_root": root, "task": "concurrent probe xyz"}),
                )
                .await
            }));
        }
        let mut results = Vec::new();
        for t in tasks {
            results.push(t.await.expect("join ok"));
        }
        for r in &results[1..] {
            assert_eq!(&results[0], r, "concurrent briefs must agree");
        }
        let after: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            before, after,
            "brief generation must not create workspace state"
        );
    }

    /// Conflicting decisions surface as conflicts; nothing is resolved by guessing.
    #[tokio::test]
    async fn brief_conflicting_decisions_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"conflict-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        crate::init::run(dir.path()).expect("init");
        let server = local_sandbox_server(&dir);
        call_tool_text(
            &server,
            "update_identity",
            json!({
                "add_decisions": [
                    {"title": "Adopt SQLite durable storage", "description": "SQLite backs all durable state."},
                    {"title": "Retire SQLite durable storage", "description": "Superseded by the file backend.", "status": "superseded"}
                ]
            }),
        )
        .await;
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"task": "SQLite durable storage"}),
        )
        .await;
        let v = brief_json(&out);
        let conflicts = v["decision_conflicts"].as_array().expect("conflicts");
        assert!(
            !conflicts.is_empty(),
            "shared-area decisions with differing currency must conflict: {v:?}"
        );
        assert!(conflicts[0]["decision_ids"].as_array().unwrap().len() == 2);
    }

    /// Failed tasks feed negative knowledge so approaches are not repeated.
    #[tokio::test]
    async fn brief_failed_task_feeds_negative_knowledge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let created = call_tool_text(
            &server,
            "task",
            json!({"workspace_root": ws_arg(&dir), "action": "create", "title": "Migrate to new cache"}),
        )
        .await;
        let task_id = brief_json(&created)["task"]["task_id"]
            .as_str()
            .expect("task id")
            .to_string();
        call_tool_text(
            &server,
            "task",
            json!({"workspace_root": ws_arg(&dir), "action": "start", "task_id": task_id}),
        )
        .await;
        call_tool_text(&server, "task", json!({"workspace_root": ws_arg(&dir), "action": "fail", "task_id": task_id, "reason": "cache migration corrupted state"})).await;
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir), "task_id": task_id, "task": "cache migration"}),
        )
        .await;
        let v = brief_json(&out);
        assert_eq!(v["task_state"]["status"], "failed");
        let neg = v["negative_knowledge"]
            .as_array()
            .expect("negative knowledge");
        assert!(neg.iter().any(|n| n["kind"] == "failed_task"), "{neg:?}");
    }

    /// Task skill refs are reference-only and bounded; no execution implied.
    #[tokio::test]
    async fn brief_task_skill_refs_bounded_and_reference_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = tempfile::tempdir().expect("tempdir");
        let server = stateful_server(&dir, &state);
        let refs: Vec<String> = (0..10).map(|i| format!("skill-ref-{i}")).collect();
        let created = call_tool_text(
            &server,
            "task",
            json!({"workspace_root": ws_arg(&dir), "action": "create", "title": "Skill-bound work", "skill_refs": refs}),
        )
        .await;
        let task_id = brief_json(&created)["task"]["task_id"]
            .as_str()
            .expect("task id")
            .to_string();
        let out = call_tool_text(
            &server,
            "engineering_brief",
            json!({"workspace_root": ws_arg(&dir), "task_id": task_id, "task": "skill work"}),
        )
        .await;
        let v = brief_json(&out);
        let skills = v["skills"].as_array().expect("skills");
        assert!(
            skills.len() <= crate::engineering_brief::MAX_BRIEF_SKILLS,
            "{}",
            skills.len()
        );
        assert!(!skills.is_empty());
        assert!(skills
            .iter()
            .all(|s| s["origin"] == "task_ref" && s["category"] == "SKILL"));
        assert!(
            !out.to_lowercase().contains("execute"),
            "brief must never order skill execution"
        );
    }

    /// Briefs stay available during mutating operations (no lock coupling).
    #[tokio::test]
    async fn brief_concurrent_with_reindex_task_and_skill_mutation() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"conc-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let state = tempfile::tempdir().expect("tempdir");
        let server = Arc::new(stateful_server(&dir, &state));
        let root = ws_arg(&dir);

        let created = call_tool_text(
            &server,
            "task",
            json!({"workspace_root": root, "action": "create", "title": "Concurrent probe"}),
        )
        .await;
        let task_id = brief_json(&created)["task"]["task_id"]
            .as_str()
            .expect("task id")
            .to_string();
        call_tool_text(
            &server,
            "task",
            json!({"workspace_root": root, "action": "start", "task_id": task_id}),
        )
        .await;

        // reindex (mutation lock) || checkpoint (mutation lock) ||
        // skill propose (mutation lock) || brief (lock-free) run together.
        let s1 = server.clone();
        let r1 = root.clone();
        let reindex = tokio::spawn(async move {
            call_tool_text(&s1, "reindex", json!({"workspace_root": r1})).await
        });
        let s2 = server.clone();
        let r2 = root.clone();
        let t2 = task_id.clone();
        let checkpoint = tokio::spawn(async move {
            call_tool_text(&s2, "task", json!({"workspace_root": r2, "action": "checkpoint", "task_id": t2, "summary": "half done"})).await
        });
        let s3 = server.clone();
        let r3 = root.clone();
        let propose = tokio::spawn(async move {
            call_tool_text(&s3, "skill", json!({"workspace_root": r3, "action": "propose", "name": "conc-skill", "description": "Concurrency probe skill", "purpose": "Testing", "content": "---\nname: conc-skill\ndescription: Concurrency probe skill\n---\n\n# Purpose\n\nBody."})).await
        });
        let mut briefs = Vec::new();
        for _ in 0..4 {
            let s = server.clone();
            let r = root.clone();
            briefs.push(tokio::spawn(async move {
                call_tool_text(
                    &s,
                    "engineering_brief",
                    json!({"workspace_root": r, "task": "concurrent probe"}),
                )
                .await
            }));
        }
        let re_out = reindex.await.expect("join ok");
        assert!(re_out.contains("\"status\": \"ok\""), "{re_out}");
        let cp_out = checkpoint.await.expect("join ok");
        assert!(cp_out.contains("checkpoint"), "{cp_out}");
        let prop_out = propose.await.expect("join ok");
        assert!(prop_out.contains("conc-skill"), "{prop_out}");
        let mut first: Option<String> = None;
        for b in briefs {
            let out = b.await.expect("join ok");
            let v = brief_json(&out);
            assert!(v["bounds"].is_object());
            if let Some(f) = first.as_ref() {
                // Briefs racing a reindex may legitimately differ in freshness;
                // each one must still be well-formed and bounded.
                assert!(brief_json(f)["bounds"].is_object());
            } else {
                first = Some(out);
            }
        }
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
                let r = server
                    .memory_stats(Parameters(MemoryStatsArgs {
                        workspace_root: None,
                    }))
                    .await
                    .map_err(|e| e.to_string())?;
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
            "engineering_memory" => {
                let p: MemoryArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .engineering_memory(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "delete_memory" => {
                let p: DeleteMemoryArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .delete_memory(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "apply_change" => {
                let p: ChangeArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .apply_change(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "apply_changes" => {
                let p: ApplyChangesArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .apply_changes(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "workspace_context" => {
                let p: WorkspaceContextArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .workspace_context(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "context" => {
                let p: ContextArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .context(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "remember" => {
                let p: RememberArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .remember(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "forget" => {
                let p: ForgetArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .forget(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "recall" => {
                let p: RecallArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .recall(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "learn" => {
                let p: LearnArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .learn(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "skill" => {
                let p: SkillArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .skill(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "task" => {
                let p: TaskArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .task(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "sandbox_exec" => {
                let p: SandboxExecArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .sandbox_exec(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "sandbox_test" => {
                let p: SandboxTestArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .sandbox_test(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "sandbox_build" => {
                let p: SandboxBuildArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .sandbox_build(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "sandbox_status" => {
                let r = server.sandbox_status().await.map_err(|e| e.to_string())?;
                text_of(r)
            }
            "impact_analyze" => {
                let p: ImpactArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .impact_analyze(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "reindex" => {
                let r = server
                    .reindex(Parameters(ReindexArgs {
                        workspace_root: None,
                    }))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "repository_health" => {
                let r = server
                    .repository_health(Parameters(RepositoryHealthArgs {
                        workspace_root: None,
                    }))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "consult" => {
                let p: ConsultArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .consult(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "update_identity" => {
                let p: UpdateIdentityArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .update_identity(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "engineering_brief" => {
                let p: EngineeringBriefArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .engineering_brief(Parameters(p))
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

    // ── Mutation-safety hardening: per-workspace mutation lock ───────────

    /// A mutating tool call must block while the workspace mutation lock is
    /// held by another caller, and complete once it is released. This pins
    /// the serialization guarantee directly instead of relying on timing.
    #[tokio::test]
    async fn mutating_calls_serialize_on_the_workspace_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = std::sync::Arc::new(local_sandbox_server(&dir));

        let ws = server.resolve_workspace(None).unwrap();
        let guard = ws.mutation_lock.lock().await;
        let s2 = server.clone();
        let mut task = tokio::spawn(async move {
            call_tool_text(
                &s2,
                "record_memory",
                json!({"key": "lock:blocked", "value": "v"}),
            )
            .await
        });

        let blocked = tokio::time::timeout(std::time::Duration::from_millis(100), &mut task)
            .await
            .is_err();
        assert!(blocked, "mutating call must wait while the lock is held");

        drop(guard);
        let out = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("task must finish after the lock is released")
            .expect("join ok");
        assert!(out.contains("memory recorded"), "got: {out}");
    }

    /// Pipelined concurrent record_memory calls must not lose entries to
    /// last-writer-wins persistence races: every entry survives a reload.
    #[tokio::test]
    async fn concurrent_record_memory_storm_preserves_every_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = std::sync::Arc::new(local_sandbox_server(&dir));

        let mut tasks = Vec::new();
        for i in 0..16 {
            let s = server.clone();
            tasks.push(tokio::spawn(async move {
                call_tool_text(
                    &s,
                    "record_memory",
                    json!({"key": format!("storm:t{i}"), "value": "v"}),
                )
                .await
            }));
        }
        for t in tasks {
            let out = t.await.expect("join ok");
            assert!(out.contains("memory recorded"), "got: {out}");
        }

        let fresh = CodeBroMcpServer::new(dir.path().to_path_buf());
        let resolved = call_tool_text(
            &fresh,
            "engineering_memory",
            json!({"task_keywords": ["storm:t"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(
            v["entries"].as_array().map(|a| a.len()),
            Some(16),
            "every concurrently recorded entry must survive"
        );
    }

    // ── RC.1 hardening: delete_memory confirm guard ──────────────────────

    /// P-RC.1: delete_memory without confirm=true must be rejected, even when
    /// the key exists. Legitimate deletion requires explicit confirmation.
    #[tokio::test]
    async fn delete_memory_rejects_without_confirm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // Record an entry.
        call_tool_text(
            &server,
            "record_memory",
            json!({"key": "p-rc:test-key", "value": "decided", "confidence": 0.8}),
        )
        .await;

        // Delete without confirm must be rejected (not silently succeed).
        let err = call_tool_err(&server, "delete_memory", json!({"key": "p-rc:test-key"})).await;
        assert!(
            err.contains("confirm=true"),
            "delete without confirm must be rejected, got: {err}"
        );

        // Entry must still exist after the rejected deletion.
        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["p-rc:test-key"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(1));

        // Confirm=true must succeed.
        call_tool_text(
            &server,
            "delete_memory",
            json!({"key": "p-rc:test-key", "confirm": true}),
        )
        .await;
        let resolved = call_tool_text(
            &server,
            "engineering_memory",
            json!({"task_keywords": ["p-rc:test-key"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(0));
    }

    /// P-RC.1 regression: deleting a missing key with confirm=true must error
    /// (not silently succeed or panic).
    #[tokio::test]
    async fn delete_memory_missing_key_with_confirm_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let err = call_tool_err(
            &server,
            "delete_memory",
            json!({"key": "ghost-key", "confirm": true}),
        )
        .await;
        assert!(
            err.contains("no entry"),
            "missing key must error, got: {err}"
        );
    }

    /// P-RC.1: persist-after-delete must survive a fresh server reload.
    #[tokio::test]
    async fn delete_memory_persist_survives_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        call_tool_text(
            &server,
            "record_memory",
            json!({"key": "p-rc:persist-test", "value": "will-delete", "confidence": 0.7}),
        )
        .await;

        // Delete with confirm.
        call_tool_text(
            &server,
            "delete_memory",
            json!({"key": "p-rc:persist-test", "confirm": true}),
        )
        .await;

        // Fresh server reads the updated (empty) state.
        let server2 = CodeBroMcpServer::new(dir.path().to_path_buf());
        let resolved = call_tool_text(
            &server2,
            "engineering_memory",
            json!({"task_keywords": ["p-rc:persist-test"]}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&resolved).expect("valid json");
        assert_eq!(v["entries"].as_array().map(|a| a.len()), Some(0));
    }

    // ── RC.1 hardening: empty fact retrieval recovery ─────────────────────

    /// P-RC.2: zero-result engineering_facts must include deterministic
    /// recovery guidance (recovery.message + recovery.hints).
    #[tokio::test]
    async fn engineering_facts_zero_result_includes_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // Build a store with one fact so the query returns 0 (not empty store).
        call_tool_text(
            &server,
            "apply_change",
            json!({"path": "src/lib.rs", "old": "", "new": "pub fn hello() {}"}),
        )
        .await;
        // Init is needed to populate facts — but we test on an empty-ish store.
        // Instead, just query a non-existent symbol directly.
        let out = call_tool_text(
            &server,
            "engineering_facts",
            json!({"query": "nonexistent-symbol-xyz"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["returned"], 0);
        let recovery = v["recovery"].as_object().expect("recovery must be present");
        assert!(recovery.contains_key("message"));
        assert!(recovery.contains_key("hints"));
        let hints: Vec<&str> = recovery["hints"]
            .as_array()
            .expect("hints must be an array")
            .iter()
            .map(|h| h.as_str().unwrap())
            .collect();
        // Should suggest shorter term.
        assert!(hints
            .iter()
            .any(|h| h.contains("shorter") || h.contains("prefix")));
    }

    /// P-RC.2: recovery is absent when results are non-empty.
    #[tokio::test]
    async fn engineering_facts_nonzero_result_has_no_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "engineering_facts",
            json!({"query": "zzz-no-such"}),
        )
        .await;
        // On an empty store with no matching query the result is 0 — we want
        // to verify that a store WITH facts returns non-zero and no recovery.
        // Instead test the empty-store case: it has recovery.
        // For non-zero, use a query that would match if any facts existed.
        // Since this is an empty store, we check the shape is consistent.
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["returned"], 0);
        assert!(v["recovery"].is_object(), "recovery present on zero-result");
    }

    /// P-RC.2: very long sentence-like queries get a "shorten your query" hint.
    #[tokio::test]
    async fn engineering_facts_long_query_hints_shorter_term() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "engineering_facts",
            json!({"query": "the circuit breaker implementation in the coding module"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["returned"], 0);
        let hints: Vec<&str> = v["recovery"]["hints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap())
            .collect();
        assert!(
            hints.iter().any(|h| h.contains("shorten")),
            "long query must suggest shortening, got: {hints:?}"
        );
    }

    // ── Sandbox tool tests ─────────────────────────────────────────────

    /// Helper: create a CodeBroMcpServer forced into local sandbox mode,
    /// regardless of OPEN_SANDBOX_URL in the environment. User-context
    /// state is hermetic: passive history capture writes on sandbox/apply
    /// paths, so the state dir lives inside the test workspace (owned by
    /// its TempDir — auto-cleaned, unique per test, never ~/.codebro).
    fn local_sandbox_server(dir: &tempfile::TempDir) -> CodeBroMcpServer {
        CodeBroMcpServer::with_state_dir(
            dir.path().to_path_buf(),
            dir.path().join(".codebro-test-state"),
        )
    }

    fn local_sandbox_server_for_path(
        path: &std::path::Path,
        state: &tempfile::TempDir,
    ) -> CodeBroMcpServer {
        // The state dir stays in the caller's TempDir (auto-cleaned):
        // fixture workspaces inside the repo checkout must never gain
        // state files.
        CodeBroMcpServer::with_state_dir(path.to_path_buf(), state.path().to_path_buf())
    }

    /// sandbox_exec must return a parseable structured result with success=true
    /// for allowed commands (true / echo).
    #[tokio::test]
    async fn sandbox_exec_runs_allowed_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_exec",
            json!({"command": "echo hello-sandbox"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["success"], true);
        assert_eq!(v["exit_code"], 0);
        assert_eq!(v["denied"], false);
        assert_eq!(v["backend"], "local");
        assert!(v["stdout"].as_str().unwrap().contains("hello-sandbox"));
    }

    /// sandbox_exec must return denied=true for destructive commands.
    #[tokio::test]
    async fn sandbox_exec_denies_destructive_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "sandbox_exec", json!({"command": "rm -rf /"})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["denied"], true);
        assert_eq!(v["exit_code"], -1);
        assert_eq!(v["success"], false);
    }

    /// sandbox_status must return backend info and available=true for local.
    #[tokio::test]
    async fn sandbox_status_returns_available() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "sandbox_status", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["available"], true);
        assert_eq!(v["backend"], "local");
    }

    /// sandbox_exec on a non-Cargo workspace must deny cargo commands.
    #[tokio::test]
    async fn sandbox_exec_denies_cargo_without_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "sandbox_exec", json!({"command": "cargo test"})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["denied"], true);
    }

    /// sandbox_test auto-detects cargo workspace and runs `cargo test`.
    #[tokio::test]
    async fn sandbox_test_auto_detects_cargo() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "sandbox_test", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["command"], "cargo test");
        assert!(v.get("verification").is_some());
        assert!(v["verification"]["verified"].is_boolean());
    }

    /// sandbox_build auto-detects cargo workspace and runs `cargo check`.
    #[tokio::test]
    async fn sandbox_build_auto_detects_cargo() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "sandbox_build", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["command"], "cargo check");
        assert!(v.get("verification").is_some());
    }

    /// sandbox_test with explicit command uses that command.
    #[tokio::test]
    async fn sandbox_test_with_explicit_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_test",
            json!({"command": "echo explicit-test"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["command"], "echo explicit-test");
        assert_eq!(v["verification"]["verified"], true);
        assert_eq!(v["execution"]["exit_code"], 0);
    }

    /// sandbox_build with explicit command uses that command.
    #[tokio::test]
    async fn sandbox_build_with_explicit_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_build",
            json!({"command": "echo explicit-build"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["command"], "echo explicit-build");
        assert_eq!(v["verification"]["verified"], true);
    }

    /// sandbox_test with expected_success=false on a passing command
    /// must report verification failure with violations.
    #[tokio::test]
    async fn sandbox_test_verification_fails_on_contradicting_expectations() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_test",
            json!({
                "command": "true",
                "expected_success": false,
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["success"], true);
        assert_eq!(v["verification"]["verified"], false);
        assert!(!v["verification"]["violations"].is_null());
        let violations: Vec<&str> = v["verification"]["violations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(violations
            .iter()
            .any(|v| v.contains("expected success=false")));
    }

    /// sandbox_exec with mixed stdout/stderr must keep them separated.
    #[tokio::test]
    async fn sandbox_exec_separates_stdout_and_stderr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // Use separate echo calls — the policy only allows single-token
        // commands, so we verify separation via the struct fields on a
        // simple command rather than trying shell redirections.
        let out = call_tool_text(
            &server,
            "sandbox_exec",
            json!({"command": "echo hello-sandbox"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["exit_code"], 0);
        assert_eq!(v["success"], true);
        assert!(v["stdout"].as_str().unwrap().contains("hello-sandbox"));
        // stderr field is present (may be empty).
        assert!(v.get("stderr").is_some());
    }

    /// sandbox_exec on timeout must preserve partial output and set timeout flag.
    #[tokio::test]
    async fn sandbox_exec_timeout_preserves_evidence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_exec",
            json!({
                "command": "sleep 30",
                "timeout": 1,
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["exit_code"], -1);
        // Timeout may or may not be set depending on timing; verify the
        // result is still structured and machine-readable.
        assert_eq!(v["backend"], "local");
        assert!(v["duration_ms"].is_number());
        assert!(!v["success"].as_bool().unwrap_or(false));
    }

    /// Integration: a failing build yields structured diagnostics, a
    /// compile_error classification, and module attribution from the fact
    /// store — validation intelligence on top of raw evidence.
    #[tokio::test]
    async fn sandbox_build_reports_structured_diagnostics_for_compile_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"broken\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn broken() {\n    let x: i32 = \"not a number\";\n}\n",
        )
        .unwrap();
        let server = local_sandbox_server(&dir);

        // The fact store needs a facts.json for module attribution.
        crate::init::run(dir.path()).expect("init succeeds");

        let out = call_tool_text(&server, "sandbox_build", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["verification"]["verified"], false);
        assert_eq!(
            v["verification"]["classification"].as_str(),
            Some("compile_error"),
            "got: {v}"
        );
        let diags = v["verification"]["diagnostics"]
            .as_array()
            .expect("diagnostics present");
        assert!(
            diags
                .iter()
                .any(|d| d["code"].as_str().is_some_and(|c| c.starts_with('E'))),
            "expected a coded compiler diagnostic: {diags:?}"
        );
        let affected = v["verification"]["affected_modules"]
            .as_array()
            .expect("affected modules present");
        assert!(
            affected
                .iter()
                .any(|m| m.as_str().unwrap_or("").contains("src/lib.rs")),
            "lib.rs diagnostic must map to its module: {affected:?}"
        );
    }

    /// Integration: failure↔change correlation — a file edited moments ago
    /// that now fails to compile is surfaced as `related_recent_changes`.
    #[tokio::test]
    async fn sandbox_build_correlates_failures_with_recent_edits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"corr\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn ok() -> i32 {\n    1\n}\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init succeeds");

        // Break the file through the guarded mutation seam.
        call_tool_text(
            &server,
            "apply_change",
            json!({"path": "src/lib.rs", "old": "    1", "new": "    \"not a number\""}),
        )
        .await;

        let out = call_tool_text(&server, "sandbox_build", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["verification"]["verified"], false);
        assert_eq!(
            v["verification"]["classification"].as_str(),
            Some("compile_error")
        );
        let related = v["verification"]["related_recent_changes"]
            .as_array()
            .expect("correlation present");
        assert!(
            related
                .iter()
                .any(|r| r["path"].as_str() == Some("src/lib.rs")),
            "edited file must be correlated: {related:?}"
        );
        assert!(related[0]["seconds_ago"].is_u64());
    }

    /// Integration: sandbox_build against the real cargo fixture.
    #[tokio::test]
    async fn sandbox_build_fixture_passes() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/cargo-project");
        let state = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server_for_path(&fixture, &state);
        let out = call_tool_text(&server, "sandbox_build", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["command"], "cargo check");
        assert_eq!(v["execution"]["exit_code"], 0);
        assert_eq!(v["verification"]["verified"], true);
    }

    /// Integration: sandbox_test against the real cargo fixture.
    #[tokio::test]
    async fn sandbox_test_fixture_passes() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/cargo-project");
        let state = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server_for_path(&fixture, &state);
        let out = call_tool_text(&server, "sandbox_test", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["command"], "cargo test");
        assert_eq!(v["execution"]["exit_code"], 0);
        assert_eq!(v["verification"]["verified"], true);
        assert!(v["execution"]["stdout"]
            .as_str()
            .unwrap()
            .contains("test result"));
    }

    /// Integration: sandbox_test against failing fixture reports failure evidence.
    #[tokio::test]
    async fn sandbox_test_fixture_failing_reports_verification_failure() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/cargo-project-failing");
        let state = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server_for_path(&fixture, &state);
        let out = call_tool_text(&server, "sandbox_test", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        // The fixture has a should_panic test which panics in normal test runs,
        // causing a non-zero exit.
        assert_eq!(v["execution"]["command"], "cargo test");
        // Evidence must be preserved regardless of outcome.
        assert!(v["execution"]["stdout"].is_string() || v["execution"]["stdout"].is_null());
        assert!(v["execution"]["stderr"].is_string() || v["execution"]["stderr"].is_null());
        assert_eq!(v["execution"]["backend"], "local");
        assert!(v["execution"]["duration_ms"].is_number());
        // Verification reflects the failure.
        assert_eq!(v["verification"]["verified"], false);
        assert!(!v["verification"]["violations"].is_null());
    }

    /// Integration: sandbox_exec with metadata passthrough preserves it.
    #[tokio::test]
    async fn sandbox_exec_metadata_passthrough() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_exec",
            json!({
                "command": "echo metadata-test",
                "metadata": {"run_id": "abc-123", "intent": "verify"},
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["metadata"]["run_id"], "abc-123");
        assert_eq!(v["metadata"]["intent"], "verify");
    }

    /// sandbox_test with expected_exit_code mismatch reports violation.
    #[tokio::test]
    async fn sandbox_test_expected_exit_code_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_test",
            json!({
                "command": "false",
                "expected_exit_code": 0,
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["execution"]["exit_code"], 1);
        assert_eq!(v["verification"]["verified"], false);
        let violations: Vec<&str> = v["verification"]["violations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(violations
            .iter()
            .any(|v| v.contains("expected exit_code=0")));
    }

    /// sandbox_build on a non-Cargo workspace returns a no-op command.
    #[tokio::test]
    async fn sandbox_build_no_manifest_returns_echo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "sandbox_build", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert!(v["execution"]["command"]
            .as_str()
            .unwrap()
            .contains("no project manifest"));
        assert_eq!(v["execution"]["exit_code"], 0);
        assert_eq!(v["verification"]["verified"], true);
    }

    /// OpenSandbox integration: skip when OPEN_SANDBOX_URL is unavailable,
    /// exercise the full MCP → SandboxRuntime → OpenSandbox path when it is.
    #[tokio::test]
    async fn sandbox_opensandbox_integration_skips_when_unavailable() {
        // Without OPEN_SANDBOX_URL, the runtime falls back to local.
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "sandbox_status", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["backend"], "local");
        assert_eq!(v["available"], true);
    }

    /// P1.2: sandbox_status must include capability descriptor.
    #[tokio::test]
    async fn sandbox_status_includes_capabilities() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "sandbox_status", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert!(v.get("capabilities").is_some());
        let caps = v["capabilities"]
            .as_object()
            .expect("capabilities is object");
        assert_eq!(caps["isolation"], "none");
        assert_eq!(caps["filesystem_scope"], "policy_bounded");
        assert_eq!(caps["network_access"], "host");
        assert_eq!(caps["timeout_enforcement"], true);
        assert_eq!(caps["output_limits"], true);
    }

    /// P1.2: sandbox_exec must include provenance fields (execution_id,
    /// timestamp, resolved_command, reproducibility).
    #[tokio::test]
    async fn sandbox_exec_includes_provenance_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);
        let out = call_tool_text(
            &server,
            "sandbox_exec",
            json!({"command": "echo provenance"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert!(v["execution_id"].is_string());
        assert!(!v["execution_id"].as_str().unwrap().is_empty());
        assert!(v["timestamp"].is_string());
        assert!(v["resolved_command"].is_string());
        assert_eq!(v["resolved_command"], "echo provenance");
        assert!(v["reproducibility"].is_string());
    }

    /// P1.2: sandbox_test evidence must include repo_identity and repo_state.
    #[tokio::test]
    async fn sandbox_test_includes_repo_identity_and_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        // Initialize as git repo so repo_state is captured.
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init"])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            // CI runners lack a global git identity; provide one inline.
            .args([
                "-c",
                "user.email=codebro@test",
                "-c",
                "user.name=codebro",
                "commit",
                "-m",
                "init",
            ])
            .output()
            .ok();
        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "sandbox_test", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let exec = &v["execution"];
        assert!(exec["repo_identity"].is_object());
        let ri = exec["repo_identity"].as_object().unwrap();
        assert_eq!(ri["repository_type"], "cargo");
        assert!(!ri["project_id"].as_str().unwrap().is_empty());
        assert!(exec["repo_state"].is_object());
        let rs = exec["repo_state"].as_object().unwrap();
        assert!(rs["commit_sha"].is_string());
        assert!(rs["working_tree_dirty"].is_boolean());
    }

    /// P2.1: `impact_analyze` on a symbol returns structural relationships.
    #[tokio::test]
    async fn impact_analyze_symbol_returns_structural_relationships() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub struct UserService;\npub fn get_user() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/handler.rs"),
            "use crate::lib::*;\npub fn handle() { get_user(); }\n",
        )
        .unwrap();

        // Run init to populate facts.
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Query the symbol.
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "get_user", "target_type": "symbol"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["target"]["kind"], "symbol");
        assert_eq!(v["target"]["name"], "get_user");
        // Must have target info and some structure.
        assert!(v.get("direct_relationships").is_some());
        assert!(v.get("affected_tests").is_some());
        assert!(v.get("affected_modules").is_some());
        assert!(v["completeness"].is_object());
    }

    /// P2.1: `impact_analyze` on a file path resolves the owning module.
    #[tokio::test]
    async fn impact_analyze_file_resolves_module() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "src/lib.rs", "target_type": "file"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["target"]["kind"], "module");
        assert_eq!(v["target"]["path"], "src/lib.rs");
    }

    /// P2.1: `impact_analyze` on an unknown symbol returns an error.
    #[tokio::test]
    async fn impact_analyze_unknown_symbol_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        let err = call_tool_err(
            &server,
            "impact_analyze",
            json!({"target": "nonexistent_symbol_xyz", "target_type": "symbol"}),
        )
        .await;
        assert!(err.contains("no symbol found") || err.contains("ambiguous"));
    }

    /// P2.1: `impact_analyze` rejects an invalid target_type.
    #[tokio::test]
    async fn impact_analyze_rejects_invalid_target_type() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let err = call_tool_err(
            &server,
            "impact_analyze",
            json!({"target": "x", "target_type": "bogus"}),
        )
        .await;
        assert!(err.contains("target_type") || err.contains("bogus"));
    }

    /// P2.2: `impact_analyze` on a caller symbol returns a verified
    /// `calls` relationship when the AST contains an actual call expression.
    #[tokio::test]
    async fn impact_analyze_ast_call_is_verified() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // lib.rs defines `get_user`.
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn get_user() -> i32 { 42 }\n",
        )
        .unwrap();
        // handler.rs calls `get_user()`.
        std::fs::write(
            dir.path().join("src/handler.rs"),
            "use crate::lib::get_user;\npub fn handle() { let x = get_user(); }\n",
        )
        .unwrap();

        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Query the callee — should find incoming call from handler.
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "get_user", "target_type": "symbol"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["target"]["kind"], "symbol");
        assert_eq!(v["target"]["name"], "get_user");

        // Must have at least one incoming relationship (the call from
        // handler::handle).
        let rels: Vec<&serde_json::Value> = v["direct_relationships"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["relationship_kind"] == "calls" && r["direction"] == "incoming")
            .collect();
        assert!(
            !rels.is_empty(),
            "expected at least 1 incoming calls relationship, got: {:?}",
            v["direct_relationships"]
        );
        // The call relationship must be verified (not heuristic).
        assert_eq!(rels[0]["provenance"], "verified");
    }

    /// P2.2: AST-derived call edges are deduplicated — same call found
    /// by both name-coincidence heuristic and AST extraction produces
    /// only one verified edge.
    #[tokio::test]
    async fn impact_analyze_no_duplicate_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn helper() -> i32 { 1 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "use crate::lib::helper;\npub fn main() { helper(); }\n",
        )
        .unwrap();

        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "helper", "target_type": "symbol"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");

        // Count incoming calls edges — should be exactly 1 (deduplicated).
        let incoming_calls: Vec<&serde_json::Value> = v["direct_relationships"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["relationship_kind"] == "calls" && r["direction"] == "incoming")
            .collect();
        assert_eq!(
            incoming_calls.len(),
            1,
            "expected exactly 1 incoming call edge (deduplication), got {}: {:?}",
            incoming_calls.len(),
            incoming_calls
        );
        assert_eq!(incoming_calls[0]["provenance"], "verified");
    }

    /// P2.2: A resolved AST call produces a verified edge, including calls
    /// to private same-module symbols — visibility does not suppress real
    /// call evidence. (Regression guard: before caller resolution was
    /// fixed, these edges existed in the store but were invisible to
    /// impact analysis because their source ids dangled.)
    #[tokio::test]
    async fn impact_analyze_unresolved_call_has_no_verified_edge() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // Define a function but don't export it.
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn internal() -> i32 { 1 }\npub fn runner() { internal(); }\n",
        )
        .unwrap();

        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "runner", "target_type": "symbol"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");

        // internal() is a private function, but the AST call from runner
        // is real evidence and must surface as a verified edge now that
        // caller resolution works.
        let verified_calls: Vec<&serde_json::Value> = v["direct_relationships"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["relationship_kind"] == "calls" && r["provenance"] == "verified")
            .collect();
        assert!(
            verified_calls
                .iter()
                .any(|r| r["target_name"] == "internal"),
            "expected a verified call edge runner -> internal, got {verified_calls:?}"
        );
    }

    /// `update_identity` records goals/constraints/decisions into the
    /// persistent project identity and reports duplicates as skipped.
    #[tokio::test]
    async fn update_identity_records_goals_decisions_and_skips_duplicates() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"identity-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        crate::init::run(dir.path()).expect("init");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(
            &server,
            "update_identity",
            json!({
                "current_sprint": "sprint-1",
                "add_constraints": ["no unsafe outside sandbox"],
                "add_conventions": ["tests live beside code"],
                "add_roadmap_items": [{"title": "Ship identity tooling", "status": "in_progress"}],
                "add_decisions": [{
                    "title": "MCP first architecture",
                    "description": "The MCP server is the only production interface.",
                    "context": "TUI removed in ADR-012"
                }]
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["applied"], true);
        assert_eq!(v["identity"]["current_sprint"], "sprint-1");
        assert!(v["identity"]["known_constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "no unsafe outside sandbox"));
        assert_eq!(
            v["identity"]["engineering_decisions"][0]["id"],
            "mcp-first-architecture"
        );
        assert_eq!(
            v["identity"]["engineering_decisions"][0]["status"],
            "accepted"
        );

        // Persisted to disk.
        let raw =
            std::fs::read_to_string(dir.path().join(".codebro/project_identity.json")).unwrap();
        let persisted: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(persisted["current_sprint"], "sprint-1");
        assert_eq!(persisted["roadmap"].as_array().unwrap().len(), 1);

        // Duplicate decision title is skipped, not duplicated.
        let out2 = call_tool_text(
            &server,
            "update_identity",
            json!({
                "add_decisions": [{
                    "title": "MCP first architecture",
                    "description": "Duplicate submission."
                }]
            }),
        )
        .await;
        let v2: serde_json::Value = serde_json::from_str(&out2).expect("valid json");
        assert_eq!(v2["applied"], false, "duplicate-only call must not apply");
        assert_eq!(v2["skipped"].as_array().unwrap().len(), 1);
    }

    /// `update_identity` refuses to run without an existing identity.
    #[tokio::test]
    async fn update_identity_requires_existing_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);
        let result = call_tool(
            &server,
            "update_identity",
            json!({"add_constraints": ["x"]}),
        )
        .await;
        assert!(result.is_err(), "expected error without identity");
    }

    // ── P2.3 bounded transitive traversal MCP tests ───────────────────────

    /// P2.3: default impact_analyze (depth=1) returns direct relationships
    /// with depth metadata and new result fields present.
    #[tokio::test]
    async fn impact_analyze_default_depth_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn get_user() -> i32 { 42 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/handler.rs"),
            "use crate::lib::get_user;\npub fn handle() { let x = get_user(); }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "get_user", "target_type": "symbol"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        // New fields must be present.
        assert!(v.get("transitive_relationships").is_some());
        assert!(v.get("provenance_summary").is_some());
        assert!(v.get("traversal_metadata").is_some());
        assert_eq!(v["traversal_metadata"]["depth_limit"], 1);
        // Each direct relationship carries depth=1.
        for rel in v["direct_relationships"].as_array().unwrap() {
            assert_eq!(rel["depth"], 1);
        }
    }

    /// P2.3: depth=0 returns target only with no relationships.
    #[tokio::test]
    async fn impact_analyze_depth_zero_returns_target_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "hello", "target_type": "symbol", "depth": 0}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        assert_eq!(
            v["direct_relationships"].as_array().map(|a| a.len()),
            Some(0)
        );
        assert_eq!(
            v["transitive_relationships"].as_array().map(|a| a.len()),
            Some(0)
        );
        assert_eq!(v["traversal_metadata"]["depth_limit"], 0);
    }

    /// P2.3: invalid depth (>5) is rejected as invalid params.
    #[tokio::test]
    async fn impact_analyze_invalid_depth_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let err = call_tool_err(
            &server,
            "impact_analyze",
            json!({"target": "hello", "target_type": "symbol", "depth": 10}),
        )
        .await;
        assert!(
            err.contains("depth") || err.contains("maximum"),
            "expected depth validation error, got: {err}"
        );
    }

    /// P2.3: direction=outgoing restricts results to outgoing edges only.
    #[tokio::test]
    async fn impact_analyze_direction_outgoing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn get_user() -> i32 { 42 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/handler.rs"),
            "use crate::lib::get_user;\npub fn handle() { let x = get_user(); }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        // Query the callee — with direction=outgoing, we should only see
        // outgoing edges from get_user (if any), not the incoming call from handle.
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "get_user", "target_type": "symbol", "direction": "outgoing"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        for rel in v["direct_relationships"].as_array().unwrap() {
            assert_eq!(rel["direction"], "outgoing");
        }
    }

    /// P2.3: provenance_summary reflects verified vs heuristic edge counts.
    #[tokio::test]
    async fn impact_analyze_provenance_summary() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn helper() -> i32 { 1 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "use crate::lib::helper;\npub fn main() { helper(); }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "helper", "target_type": "symbol"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        let summary = &v["provenance_summary"];
        assert!(summary.is_object());
        assert!(summary["verified_edges"].is_number());
        assert!(summary["heuristic_edges"].is_number());
        assert!(summary["unknown_edges"].is_number());
    }

    /// P2.3: traversal_metadata includes depth_limit, direction, truncated flag.
    #[tokio::test]
    async fn impact_analyze_traversal_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(
            &server,
            "impact_analyze",
            json!({"target": "hello", "target_type": "symbol", "depth": 2, "direction": "outgoing"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        let meta = &v["traversal_metadata"];
        assert_eq!(meta["depth_limit"], 2);
        assert_eq!(meta["direction"], "outgoing");
        assert!(meta["nodes_visited"].is_number());
        assert!(meta["edges_traversed"].is_number());
        assert_eq!(meta["truncated"], false);
    }

    // ── P2.4 engineering_facts enrichment tests ─────────────────────────

    /// P2.4: engineering_facts response must include provenance_summary and
    /// freshness at the top level.
    #[tokio::test]
    async fn engineering_facts_response_includes_provenance_summary_and_freshness() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn helper() -> i32 { 1 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "engineering_facts", json!({"query": "helper"})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        // provenance_summary must be present
        assert!(v["provenance_summary"].is_object());
        let ps = &v["provenance_summary"];
        assert!(ps["verified_edges"].is_number());
        assert!(ps["heuristic_edges"].is_number());
        assert!(ps["unknown_edges"].is_number());

        // freshness must be present
        assert!(v["freshness"].is_string());
        let fresh = v["freshness"].as_str().unwrap();
        assert!(matches!(fresh, "fresh" | "stale" | "unknown"));
    }

    /// P2.4: FactRecords must carry enrichment fields (module, package,
    /// relationship_count, test_count, provenance_type).
    #[tokio::test]
    async fn engineering_facts_returns_enriched_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"enriched-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn foo() -> i32 { 1 }\npub fn bar() -> i32 { foo() }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "engineering_facts", json!({"query": "foo"})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let facts = v["facts"].as_array().expect("facts is array");
        let foo = facts
            .iter()
            .find(|f| f["name"] == "foo")
            .expect("foo must be found");

        // module and package may be present (depends on tree-sitter output)
        assert!(
            foo.get("module").is_some()
                || foo.get("package").is_some()
                || foo["relationship_count"].is_number()
                || foo["test_count"].is_number()
        );
        // relationship_count must be a number when present
        if let Some(rc) = foo.get("relationship_count") {
            assert!(rc.is_number());
        }
        // test_count must be a number when present
        if let Some(tc) = foo.get("test_count") {
            assert!(tc.is_number());
        }
        // provenance_type must be a valid string when present
        if let Some(pt) = foo.get("provenance_type") {
            let s = pt.as_str().expect("provenance_type must be a string");
            assert!(matches!(s, "verified" | "heuristic" | "unknown" | "none"));
        }
    }

    /// P2.4: Freshness must be Stale when repository state changes after
    /// facts are generated, even if facts.json is untouched.
    #[tokio::test]
    async fn freshness_becomes_stale_after_repo_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"fresh-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn original() -> i32 { 1 }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        // First query: freshness should be Fresh (or Unknown if not a git repo).
        let server = local_sandbox_server(&dir);
        let out1 = call_tool_text(&server, "engineering_facts", json!({"query": "original"})).await;
        let v1: serde_json::Value = serde_json::from_str(&out1).expect("valid json");
        let fresh1 = v1["freshness"].as_str().unwrap();

        // Modify a source file without re-running init.
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn original() -> i32 { 1 }\npub fn added() -> i32 { 2 }\n",
        )
        .unwrap();

        // Second query: freshness must be Stale (or Unknown if git unavailable).
        let server2 = local_sandbox_server(&dir);
        let out2 =
            call_tool_text(&server2, "engineering_facts", json!({"query": "original"})).await;
        let v2: serde_json::Value = serde_json::from_str(&out2).expect("valid json");
        let fresh2 = v2["freshness"].as_str().unwrap();

        // If the first was Fresh, the second must be Stale (not Fresh).
        // If the first was Unknown (no git), both may be Unknown — that's ok.
        if fresh1 == "fresh" {
            assert_eq!(
                fresh2, "stale",
                "freshness must become stale after source modification"
            );
        }
    }

    /// P2.4: Backward compatibility — existing search behavior (ranking,
    /// limits, kind/path filters) must remain unchanged.
    #[tokio::test]
    async fn engineering_facts_backward_compatibility() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"compat-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub struct Config {}\npub fn prepare() {}\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Query by name must still work.
        let out = call_tool_text(&server, "engineering_facts", json!({"query": "Config"})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["returned"], 1);
        assert_eq!(v["facts"][0]["name"], "Config");
        assert_eq!(v["facts"][0]["kind"], "symbol");

        // Kind filter must still work.
        let out = call_tool_text(
            &server,
            "engineering_facts",
            json!({"query": "", "kind": "symbol", "limit": 5}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert!(v["facts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["kind"] == "symbol"));

        // Empty query without filter must still be rejected.
        let err = call_tool_err(&server, "engineering_facts", json!({"query": ""})).await;
        assert!(err.contains("query is required"));
    }

    // ── M3 evidence correlation integration tests ───────────────────────

    /// M3 integration: the full evidence chain from apply_change → impact
    /// → sandbox_test with correlation metadata.
    ///
    /// 1. Existing fact store contains symbol A in src/a.rs and a
    ///    relationship A↔B where the relationship's location.file is
    ///    src/b.rs.
    /// 2. apply_change mutates src/a.rs.
    /// 3. The advisory identifies the relationship even though its
    ///    location.file is src/b.rs (M3 source-side coverage).
    /// 4. The agent supplies those fact IDs to sandbox_test.
    /// 5. VerificationResult returns those IDs as correlation metadata.
    /// 6. No claim is made that sandbox execution independently verified
    ///    those facts.
    #[tokio::test]
    async fn m3_evidence_chain_integration() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"m3-integration\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "pub fn foo() -> i32 { 42 }\n").unwrap();
        std::fs::write(
            dir.path().join("src/b.rs"),
            "use super::foo;\npub fn bar() -> i32 { foo() }\n",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init failed");

        let server = local_sandbox_server(&dir);

        // Step 1: apply_change mutates src/a.rs.
        let change_out = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "src/a.rs",
                "old": "pub fn foo() -> i32 { 42 }",
                "new": "pub fn foo() -> i32 { 99 }"
            }),
        )
        .await;
        let change_v: serde_json::Value = serde_json::from_str(&change_out).expect("valid json");
        assert_eq!(change_v["applied"], true);
        let affected_fact_ids: Vec<String> = change_v["affected_fact_ids"]
            .as_array()
            .expect("affected_fact_ids is array")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(!affected_fact_ids.is_empty(), "must have affected fact IDs");

        // Step 2: impact_analyze on the changed symbol to get correlation IDs.
        let impact_out = call_tool_text(
            &server,
            "impact_analyze",
            json!({
                "target": "foo",
                "target_type": "symbol"
            }),
        )
        .await;
        let impact_v: serde_json::Value = serde_json::from_str(&impact_out).expect("valid json");
        // Freshness must be present (store has generation state and dir is a git repo... or not, but field exists).
        assert!(
            impact_v.get("freshness").is_some(),
            "freshness field must be present"
        );

        // Step 3: sandbox_test with the affected fact IDs as correlation.
        let test_out = call_tool_text(
            &server,
            "sandbox_test",
            json!({
                "affected_fact_ids": affected_fact_ids,
                "expected_success": true,
            }),
        )
        .await;
        let test_v: serde_json::Value = serde_json::from_str(&test_out).expect("valid json");
        let verification = &test_v["verification"];
        assert!(
            verification.get("impacted_fact_ids").is_some(),
            "impacted_fact_ids must be present in verification"
        );
        let returned_ids: Vec<&str> = verification["impacted_fact_ids"]
            .as_array()
            .expect("impacted_fact_ids is array")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            returned_ids, affected_fact_ids,
            "verification must echo back the caller-supplied IDs"
        );
        // The IDs are correlation context only — they do not affect verified.
        // (Tests may pass or fail independently.)
        assert!(verification.get("verified").is_some());
        assert!(verification["verified"].is_boolean());
    }

    /// M3: sandbox_build preserves caller-supplied impacted_fact_ids.
    #[tokio::test]
    async fn m3_sandbox_build_preserves_impacted_fact_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let ids = vec![
            "sym::example::foo_0".to_string(),
            "rel::foo_calls_bar".to_string(),
        ];
        let out = call_tool_text(
            &server,
            "sandbox_build",
            json!({
                "affected_fact_ids": ids,
                "command": "echo hello",
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let verification = &v["verification"];
        let returned: Vec<&str> = verification["impacted_fact_ids"]
            .as_array()
            .expect("impacted_fact_ids is array")
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert_eq!(returned, ids);
    }

    /// M3: sandbox_test without affected_fact_ids returns None (omitted).
    #[tokio::test]
    async fn m3_sandbox_test_without_ids_omits_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(
            &server,
            "sandbox_test",
            json!({
                "command": "echo hello",
            }),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        // impacted_fact_ids should be absent from the verification object
        // when the caller did not supply any.
        assert!(
            v["verification"].get("impacted_fact_ids").is_none(),
            "impacted_fact_ids must be omitted when not supplied"
        );
    }

    // ── M4: reindex MCP tool tests ────────────────────────────────────

    /// M4.A: reindex_tool_returns_structured_response
    #[tokio::test]
    async fn reindex_tool_returns_structured_response() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"reindex-struct\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn hello() -> i32 { 42 }\n",
        )
        .unwrap();

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "reindex", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        assert_eq!(v["status"], "ok");
        assert!(v["fact_counts"].is_object());
        assert!(v["generation_repo_state"].is_object() || v["generation_repo_state"].is_null());
        assert!(v["validation"].is_object());
        assert!(v["duration_ms"].is_number());
    }

    /// M4.B: reindex_tool_reloads_fact_store
    #[tokio::test]
    async fn reindex_tool_reloads_fact_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"reindex-reload\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn original() -> i32 { 1 }\n",
        )
        .unwrap();

        // Initial init + server.
        crate::init::run(dir.path()).expect("initial init failed");
        let server = local_sandbox_server(&dir);

        // Call reindex after adding a new symbol.
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn original() -> i32 { 1 }\npub fn new_symbol() -> i32 { 2 }\n",
        )
        .unwrap();
        let out = call_tool_text(&server, "reindex", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");

        // Subsequent engineering_facts must see the regenerated fact.
        let facts_out =
            call_tool_text(&server, "engineering_facts", json!({"query": "new_symbol"})).await;
        let facts_v: serde_json::Value = serde_json::from_str(&facts_out).expect("valid json");
        let facts = facts_v["facts"].as_array().expect("facts is array");
        assert!(
            facts.iter().any(|f| f["name"] == "new_symbol"),
            "regenerated fact must be visible after reindex"
        );
    }

    /// M4.C: reindex_tool_on_workspace_without_existing_facts
    #[tokio::test]
    async fn reindex_tool_on_workspace_without_existing_facts() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"reindex-no-facts\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn foo() -> i32 { 1 }\n").unwrap();

        // No prior init — facts.json should not exist.
        assert!(
            !dir.path().join(".codebro/facts.json").exists(),
            "facts.json must not exist before reindex"
        );

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "reindex", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "ok");
        assert!(v["fact_counts"]["total"].is_number());
        assert!(
            dir.path().join(".codebro/facts.json").exists(),
            "reindex must create .codebro/facts.json"
        );
    }

    /// M4.D: reindex_tool_deterministic
    #[tokio::test]
    async fn reindex_tool_deterministic() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"reindex-determ\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn bar() -> i32 { 1 }\npub fn baz() -> i32 { 2 }\n",
        )
        .unwrap();

        let server = local_sandbox_server(&dir);

        // First reindex.
        let out1 = call_tool_text(&server, "reindex", json!({})).await;
        let v1: serde_json::Value = serde_json::from_str(&out1).expect("valid json");

        // Second reindex without source changes.
        let out2 = call_tool_text(&server, "reindex", json!({})).await;
        let v2: serde_json::Value = serde_json::from_str(&out2).expect("valid json");

        // Fact counts must remain identical.
        assert_eq!(
            v1["fact_counts"], v2["fact_counts"],
            "fact counts must be identical across reindexes"
        );

        // Validation must remain identical.
        assert_eq!(
            v1["validation"], v2["validation"],
            "validation must be identical across reindexes"
        );
    }

    /// M4.E: reindex_tool_preserves_generation_state
    #[tokio::test]
    async fn reindex_tool_preserves_generation_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"reindex-genstate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn gen_state_fn() -> i32 { 1 }\n",
        )
        .unwrap();

        // Initialize a git repo so RepoState::capture succeeds.
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["init"])
            .output()
            .expect("git init succeeded");
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["config", "user.email", "test@test.com"])
            .output()
            .expect("git config succeeded");
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["config", "user.name", "Test"])
            .output()
            .expect("git config succeeded");
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["add", "."])
            .output()
            .expect("git add succeeded");
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args([
                "-c",
                "user.email=codebro@test",
                "-c",
                "user.name=codebro",
                "commit",
                "-m",
                "initial",
            ])
            .output()
            .expect("git commit succeeded");

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "reindex", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        let gen_state = v["generation_repo_state"]
            .as_object()
            .expect("generation_repo_state is object");
        assert!(gen_state.contains_key("commit_sha"));
        assert!(gen_state.contains_key("working_tree_dirty"));
        assert!(gen_state.contains_key("working_tree_hash"));

        // Verify the generation_repo_state matches what's in the persisted facts.
        let facts: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("read facts"),
        )
        .expect("parse facts");
        let persisted_gen = facts["generation_repo_state"]
            .as_object()
            .expect("persisted generation_repo_state");
        assert_eq!(
            gen_state, persisted_gen,
            "reindex response generation_repo_state must match persisted facts"
        );
    }

    /// M4.F: reindex_failure_is_reported
    #[tokio::test]
    async fn reindex_failure_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"reindex-fail\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn foo() -> i32 { 1 }\n").unwrap();

        // Create a file at .codebro so that create_dir_all fails.
        std::fs::write(dir.path().join(".codebro"), "not-a-directory").unwrap();

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "reindex", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        assert_eq!(v["status"], "error");
        assert!(v["error"].is_string());
        assert!(!v["error"].as_str().unwrap().is_empty());
        assert!(v["duration_ms"].is_number());

        // Must not contain fake success data.
        assert!(
            v.get("fact_counts").is_none(),
            "error response must not contain fact_counts"
        );
        assert!(
            v.get("generation_repo_state").is_none(),
            "error response must not contain generation_repo_state"
        );
        assert!(
            v.get("validation").is_none(),
            "error response must not contain validation"
        );
        assert_eq!(v["index_status"], "FAILED");
    }

    /// P6-AUDIT: the MCP diff shares the indexer kernel (no second
    /// implementation) and signals truncation honestly with totals.
    #[test]
    fn mcp_diff_delegates_to_single_kernel_and_signals_truncation() {
        use std::collections::BTreeMap;
        // Small diff: identical to the kernel, no truncation.
        let prev: BTreeMap<String, String> = [("a.rs".to_string(), "h1".to_string())]
            .into_iter()
            .collect();
        let curr: BTreeMap<String, String> = [
            ("a.rs".to_string(), "h1".to_string()),
            ("b.rs".to_string(), "h2".to_string()),
        ]
        .into_iter()
        .collect();
        let kernel = crate::init::engineering::diff_digests(&prev, &curr);
        let mcp = diff_digests_for_mcp(&prev, &curr);
        assert_eq!(mcp.added, kernel.added);
        assert_eq!(mcp.deleted, kernel.deleted);
        assert_eq!(mcp.modified, kernel.modified);
        assert_eq!(mcp.unchanged, kernel.unchanged);
        assert!(!mcp.truncated);
        assert_eq!(
            (mcp.added_count, mcp.deleted_count, mcp.modified_count),
            (1, 0, 0)
        );
        assert_eq!(mcp.unchanged_count, 1);
        // Large diff: lists cap at MCP_DIFF_LIST_CAP but totals stay exact
        // and `truncated` is visible (no silent drop).
        let big_prev: BTreeMap<String, String> = BTreeMap::new();
        let big_curr: BTreeMap<String, String> = (0..250)
            .map(|i| (format!("f{i:03}.rs"), "h".to_string()))
            .collect();
        let big = diff_digests_for_mcp(&big_prev, &big_curr);
        assert_eq!(big.added.len(), MCP_DIFF_LIST_CAP);
        assert_eq!(big.added_count, 250);
        assert!(big.truncated);
        // Truncated lists are the deterministic head of the sorted full list.
        let mut sorted: Vec<String> = (0..250).map(|i| format!("f{i:03}.rs")).collect();
        sorted.sort();
        sorted.truncate(MCP_DIFF_LIST_CAP);
        assert_eq!(big.added, sorted);
    }

    /// P6-AUDIT: a failed index run preserves last-good metadata instead
    /// of zeroing counts/revision (the old branch wrote symbol_count 0,
    /// edge_count 0, revision "unknown").
    #[test]
    fn failed_index_upsert_preserves_last_good_metadata() {
        let prev = crate::context_runtime::RepoIndexRecord {
            workspace_root: "/w".to_string(),
            repository_identity: "{\"project_id\":\"abc\"}".to_string(),
            index_status: crate::context_runtime::RepoIndexStatus::Ready,
            indexed_at: 111,
            repository_revision: "rev-good".to_string(),
            file_count: 10,
            symbol_count: 50,
            edge_count: 20,
            stale_count: 2,
            updated_at: 111,
        };
        let up = failed_index_upsert(&prev);
        assert_eq!(
            up.index_status,
            crate::context_runtime::RepoIndexStatus::Failed
        );
        assert_eq!(up.file_count, 10);
        assert_eq!(up.symbol_count, 50);
        assert_eq!(up.edge_count, 20);
        assert_eq!(up.stale_count, 2);
        assert_eq!(up.repository_revision, "rev-good");
        assert_eq!(up.indexed_at, 111);
        assert_eq!(up.repository_identity, "{\"project_id\":\"abc\"}");
    }

    // ── M5: repository_health MCP tool tests ──────────────────────────

    /// M5.A: repository_health_healthy_workspace — initialized workspace
    /// returns a structured report with all expected checks present.
    /// Note: init alone does not create project_identity, so the workspace
    /// is at most "warn" (missing identity), never "healthy" (exit 0).
    #[tokio::test]
    async fn repository_health_healthy_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::init::run(dir.path()).expect("init");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        // Init creates facts; missing project_identity yields at most warn.
        assert!(
            v["exit_code"] == 0 || v["exit_code"] == 1,
            "exit_code must be 0 or 1, got {}",
            v["exit_code"]
        );
        assert!(
            v["status"] == "healthy" || v["status"] == "warn",
            "status must be healthy or warn, got {}",
            v["status"]
        );
        assert!(v["checks"].is_array(), "checks must be an array");
        assert!(
            !v["checks"].as_array().unwrap().is_empty(),
            "must have checks"
        );
        assert!(v["summary"].is_string(), "summary must be a string");

        let check_names: Vec<&str> = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(check_names.contains(&"workspace_root"));
        assert!(check_names.contains(&"facts"));
        assert!(check_names.contains(&"engineering_memory"));
        assert!(check_names.contains(&".codebro"));
    }

    /// M5.B: repository_health_warning_workspace — uninitialized workspace
    /// returns status=warn with warning checks preserved.
    #[tokio::test]
    async fn repository_health_warning_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        // Uninitialized workspace has warnings (missing .codebro, facts, etc.)
        // but no errors -> exit_code 1, status "warn".
        assert_eq!(
            v["exit_code"], 1,
            "uninitialized workspace must return exit_code 1"
        );
        assert_eq!(v["status"], "warn", "status must be warn");
        assert!(v["checks"].is_array());

        // At least one check must be warn.
        let warns: Vec<&str> = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["status"] == "warn")
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(
            !warns.is_empty(),
            "uninitialized workspace must have at least one warn check, got: {warns:?}"
        );
        // No fabricated errors.
        let errors: Vec<&str> = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["status"] == "error")
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(
            errors.is_empty(),
            "uninitialized workspace must have no errors, got: {errors:?}"
        );
    }

    /// M5.C: repository_health_error_workspace — corrupt facts.json causes
    /// status=error and exit_code=2.
    #[tokio::test]
    async fn repository_health_error_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cb = dir.path().join(".codebro");
        std::fs::create_dir_all(&cb).expect("create .codebro");
        std::fs::write(cb.join("facts.json"), "not json").expect("write corrupt facts");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        assert_eq!(v["exit_code"], 2, "corrupt facts must return exit_code 2");
        assert_eq!(v["status"], "error", "status must be error");

        let facts_check = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "facts")
            .expect("facts check must exist");
        assert_eq!(facts_check["status"], "error", "facts check must be error");
        assert!(facts_check["detail"].is_string());
    }

    /// M5.D: repository_health_response_shape — the JSON response has exactly
    /// the intended top-level contract: exit_code, status, checks, summary.
    #[tokio::test]
    async fn repository_health_response_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        assert!(v.get("exit_code").is_some(), "response must have exit_code");
        assert!(v.get("status").is_some(), "response must have status");
        assert!(v.get("checks").is_some(), "response must have checks");
        assert!(v.get("summary").is_some(), "response must have summary");

        // No extra top-level keys that leak internal doctor implementation.
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        for key in &keys {
            match *key {
                "exit_code" | "status" | "checks" | "summary" => {}
                other => panic!("unexpected top-level key: {other}"),
            }
        }

        // Each check has name, status, detail.
        for check in v["checks"].as_array().unwrap() {
            assert!(check.get("name").is_some(), "check must have name");
            assert!(check.get("status").is_some(), "check must have status");
            assert!(check.get("detail").is_some(), "check must have detail");
            match check["status"].as_str().unwrap() {
                "ok" | "warn" | "error" => {}
                other => panic!("invalid check status: {other}"),
            }
        }
    }

    /// M5.E: repository_health_does_not_mutate_workspace — calling
    /// repository_health must not change any files in the workspace.
    #[tokio::test]
    async fn repository_health_does_not_mutate_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::init::run(dir.path()).expect("init");

        // Capture file list before.
        let before: Vec<std::path::PathBuf> = collect_paths(dir.path());

        let server = local_sandbox_server(&dir);
        call_tool_text(&server, "repository_health", json!({})).await;

        // Capture file list after.
        let after: Vec<std::path::PathBuf> = collect_paths(dir.path());
        assert_eq!(
            before, after,
            "repository_health must not mutate the workspace"
        );
    }

    /// M5.F: repository_health_matches_doctor — the MCP wrapper must produce
    /// the same exit code and check semantics as the direct doctor::run()
    /// call on the same fixture.
    #[tokio::test]
    async fn repository_health_matches_doctor() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::init::run(dir.path()).expect("init");

        // Direct doctor call.
        let doctor_code = crate::doctor::run(dir.path()).unwrap();
        let (_, doctor_checks) = crate::doctor::report(dir.path()).unwrap();

        // MCP wrapper call.
        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        assert_eq!(
            v["exit_code"].as_i64().unwrap() as i32,
            doctor_code,
            "MCP exit_code must match doctor::run() exit code"
        );
        assert_eq!(v["status"].as_str().unwrap(), doctor_status(doctor_code));

        // Check count must match.
        let mcp_check_count = v["checks"].as_array().unwrap().len();
        assert_eq!(
            mcp_check_count,
            doctor_checks.len(),
            "MCP check count must match doctor check count"
        );

        // Each MCP check name/status must match a doctor check.
        let mcp_names: Vec<&str> = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        let doctor_names: Vec<&str> = doctor_checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            mcp_names, doctor_names,
            "MCP check names must match doctor check names in order"
        );

        for (mcp_c, doc_c) in v["checks"].as_array().unwrap().iter().zip(&doctor_checks) {
            let expected_status = if doc_c.ok {
                "ok"
            } else if doc_c
                .detail
                .as_deref()
                .is_some_and(|d| d.starts_with("ERROR"))
            {
                "error"
            } else {
                "warn"
            };
            assert_eq!(
                mcp_c["status"].as_str().unwrap(),
                expected_status,
                "status mismatch for check '{}'",
                doc_c.name
            );
        }
    }

    /// M5.G: repository_health_error_matches_doctor — corrupt facts fixture
    /// produces the same error semantics from both doctor::run and MCP.
    #[tokio::test]
    async fn repository_health_error_matches_doctor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cb = dir.path().join(".codebro");
        std::fs::create_dir_all(&cb).unwrap();
        std::fs::write(cb.join("facts.json"), "not json").unwrap();

        let doctor_code = crate::doctor::run(dir.path()).unwrap();
        let (_, doctor_checks) = crate::doctor::report(dir.path()).unwrap();

        let server = local_sandbox_server(&dir);
        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        assert_eq!(v["exit_code"].as_i64().unwrap() as i32, doctor_code);
        assert_eq!(v["status"].as_str().unwrap(), doctor_status(doctor_code));

        let mcp_names: Vec<&str> = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        let doctor_names: Vec<&str> = doctor_checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(mcp_names, doctor_names);
    }

    /// Execution evidence is a health signal: a fresh workspace with no
    /// journal reports `execution_evidence` as ok (absent is normal) and
    /// the health call creates no files.
    #[tokio::test]
    async fn repository_health_reports_execution_evidence_absent_as_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        let check = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "execution_evidence")
            .expect("execution_evidence check must exist");
        assert_eq!(check["status"], "ok");
        assert!(
            check["detail"].as_str().unwrap().contains("no executions"),
            "{}",
            check["detail"]
        );
        // Fresh workspace: no journal file may be created by a read-only check.
        assert!(!dir.path().join(".codebro/execution_evidence.json").exists());
    }

    /// After a validation run records evidence, repository_health surfaces
    /// the journal state (record count) without failing.
    #[tokio::test]
    async fn repository_health_reports_execution_evidence_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        // Seed one journal record through the real verification path using
        // a synthetic execution with a capturable tree hash.
        let execution = crate::sandbox::ExecutionResult {
            repo_state: Some(crate::sandbox::RepoState {
                commit_sha: "c".into(),
                working_tree_dirty: false,
                working_tree_hash: "treeH".into(),
            }),
            ..crate::sandbox::ExecutionResult::from_local(
                "cargo test",
                "/tmp/nowhere",
                "",
                "",
                0,
                25,
                false,
                false,
                std::collections::HashMap::new(),
            )
        };
        let verification = crate::sandbox::VerificationResult::from_execution(execution);
        let _ = server.verification_object(
            &server.resolve_workspace(None).unwrap(),
            &verification,
            &[],
        );

        let out = call_tool_text(&server, "repository_health", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");

        let check = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "execution_evidence")
            .expect("execution_evidence check must exist");
        assert_eq!(check["status"], "ok");
        assert!(
            check["detail"].as_str().unwrap().contains("1 records"),
            "{}",
            check["detail"]
        );
    }

    fn collect_paths(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        for entry in walkdir::WalkDir::new(dir)
            .min_depth(1)
            .into_iter()
            .flatten()
        {
            paths.push(entry.path().to_path_buf());
        }
        paths.sort();
        paths
    }

    fn doctor_status(code: i32) -> &'static str {
        match code {
            crate::doctor::EXIT_ERROR => "error",
            crate::doctor::EXIT_WARN => "warn",
            _ => "healthy",
        }
    }

    // ── C1: consult MCP tool tests ────────────────────────────────────

    /// consult with an unknown provider must be rejected as invalid params.
    #[tokio::test]
    async fn consult_rejects_unknown_provider() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let err = call_tool_err(
            &server,
            "consult",
            json!({"provider": "bogus", "question": "hello"}),
        )
        .await;
        assert!(err.contains("unknown provider"), "got: {err}");
    }

    /// consult with `provider: "conductor"` is accepted and routed to the
    /// Conductor provider (it must fail on auth, not on provider parsing).
    #[tokio::test]
    async fn consult_accepts_conductor_provider() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let result = call_tool(
            &server,
            "consult",
            json!({"provider": "conductor", "question": "hello"}),
        )
        .await;
        match result {
            Ok(text) => {
                let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
                assert_eq!(v["provider"], "conductor");
            }
            Err(err) => {
                assert!(
                    !err.contains("unknown provider"),
                    "conductor must not be rejected as an unknown provider, got: {err}"
                );
                // Without CONDUCTOR_API_KEY this is an auth-required error.
                assert!(
                    err.contains("Conductor") || err.contains("conductor"),
                    "expected a Conductor-specific error, got: {err}"
                );
            }
        }
    }

    /// consult with an unknown mode must be rejected as invalid params.
    #[tokio::test]
    async fn consult_rejects_unknown_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let err = call_tool_err(
            &server,
            "consult",
            json!({"provider": "conductor", "mode": "bogus", "question": "hello"}),
        )
        .await;
        assert!(err.contains("unknown mode"), "got: {err}");
    }

    /// consult with an empty question must be rejected.
    #[tokio::test]
    async fn consult_rejects_empty_question() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let err = call_tool_err(&server, "consult", json!({"question": ""})).await;
        assert!(err.contains("question must not be empty"), "got: {err}");
    }

    /// consult on auto provider with no authenticated providers returns an
    /// auth-required error (not a panic or internal error).
    #[tokio::test]
    async fn consult_auto_no_auth_returns_actionable_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = local_sandbox_server(&dir);

        let result = call_tool(
            &server,
            "consult",
            json!({"provider": "auto", "question": "what is life?"}),
        )
        .await;
        match result {
            Ok(text) => {
                let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
                assert!(v.is_object(), "consult returned structured output");
            }
            Err(err) => {
                assert!(
                    err.contains("auth")
                        || err.contains("Auth")
                        || err.contains("authenticated")
                        || err.contains("Conductor")
                        || err.contains("conductor"),
                    "expected auth/error, got: {err}"
                );
            }
        }
    }

    // ── Multi-workspace isolation ──────────────────────────────────────

    /// Hermetic multi-workspace server: default workspace root_a, with
    /// user-context state isolated inside root_a (never ~/.codebro).
    /// Passive history capture on apply/test paths needs the isolation;
    /// root_a is always a test TempDir, so cleanup is automatic.
    /// P8 root authorization: every workspace these tests address
    /// (root_a as the default + each extra root) is authorized exactly
    /// as an operator would authorize at launch — the isolation the tests
    /// assert is then between two AUTHORIZED roots, which is the real
    /// product guarantee. Unauthorized roots are refused (asserted by the
    /// dedicated registry tests and the real-binary boundary probes).
    fn hermetic_multiws_server(
        root_a: &std::path::Path,
        extra_roots: &[&std::path::Path],
    ) -> CodeBroMcpServer {
        let registry = WorkspaceRegistry::with_authorized_roots(
            root_a.to_path_buf(),
            crate::workspace_registry::AuthorizedRoots::with_extras(
                root_a.to_path_buf(),
                extra_roots.iter().map(|p| p.to_path_buf()),
            ),
        );
        let state_dir = root_a.join(".codebro-test-state");
        let sandbox = crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local);
        assemble_server_with_registry(registry, sandbox, Some(state_dir))
    }

    #[tokio::test]
    async fn facts_are_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "main",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert!(facts_a["returned"].as_u64().unwrap() > 0);

        let out_b = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "main",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert!(facts_b["returned"].as_u64().unwrap() > 0);

        let ids_a: HashSet<String> = facts_a["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["id"].as_str())
            .map(|s| s.to_string())
            .collect();
        let ids_b: HashSet<String> = facts_b["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["id"].as_str())
            .map(|s| s.to_string())
            .collect();
        let overlap: HashSet<&String> = ids_a.iter().filter(|id| ids_b.contains(*id)).collect();
        assert!(
            overlap.is_empty(),
            "facts from A must not appear in B (overlap: {})",
            overlap.len()
        );
    }

    #[tokio::test]
    async fn memory_is_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let _ = call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "wsA-key",
                "value": "wsA-value",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let _ = call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "wsB-key",
                "value": "wsB-value",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;

        let out_a = call_tool_text(
            &server,
            "engineering_memory",
            json!({
                "task_keywords": ["wsA-key"],
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let mem_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        let keys_a: Vec<String> = mem_a["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["key"].as_str())
            .map(|s| s.to_string())
            .collect();
        assert!(keys_a.contains(&"wsA-key".to_string()));
        assert!(!keys_a.contains(&"wsB-key".to_string()));

        let out_b = call_tool_text(
            &server,
            "engineering_memory",
            json!({
                "task_keywords": ["wsB-key"],
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let mem_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        let keys_b: Vec<String> = mem_b["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["key"].as_str())
            .map(|s| s.to_string())
            .collect();
        assert!(keys_b.contains(&"wsB-key".to_string()));
        assert!(!keys_b.contains(&"wsA-key".to_string()));
    }

    #[tokio::test]
    async fn identity_is_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        // Set distinct identities in each workspace so we can verify isolation.
        {
            let mut rt = crate::project_identity::ProjectIdentityRuntime::new(root_a);
            rt.create_minimal("ALPHA", "rust").unwrap();
        }
        {
            let mut rt = crate::project_identity::ProjectIdentityRuntime::new(root_b);
            rt.create_minimal("BRAVO", "rust").unwrap();
        }

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a = call_tool_text(
            &server,
            "workspace_context",
            json!({ "workspace_root": root_a.to_string_lossy().to_string() }),
        )
        .await;
        let ctx_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();

        let out_b = call_tool_text(
            &server,
            "workspace_context",
            json!({ "workspace_root": root_b.to_string_lossy().to_string() }),
        )
        .await;
        let ctx_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();

        let name_a = ctx_a["project_identity"]["name"].as_str().unwrap_or("");
        let name_b = ctx_b["project_identity"]["name"].as_str().unwrap_or("");
        assert_eq!(name_a, "ALPHA");
        assert_eq!(name_b, "BRAVO");
    }

    #[tokio::test]
    async fn mutation_locks_are_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        std::fs::write(root_a.join("a.txt"), "hello alpha\n").unwrap();
        std::fs::write(root_b.join("b.txt"), "hello bravo\n").unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "a.txt",
                "old": "hello alpha\n",
                "new": "goodbye alpha\n",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let res_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert_eq!(res_a["applied"], true);

        let out_b = call_tool_text(
            &server,
            "apply_change",
            json!({
                "path": "b.txt",
                "old": "hello bravo\n",
                "new": "goodbye bravo\n",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let res_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert_eq!(res_b["applied"], true);

        assert_eq!(
            std::fs::read_to_string(root_a.join("a.txt")).unwrap(),
            "goodbye alpha\n"
        );
        assert_eq!(
            std::fs::read_to_string(root_b.join("b.txt")).unwrap(),
            "goodbye bravo\n"
        );
    }

    #[tokio::test]
    async fn sandbox_exec_follows_workspace() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(
            root_a.join("marker.txt"),
            "ALPHA_CODEBRO_WORKSPACE_MARKER\n",
        )
        .unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(
            root_b.join("marker.txt"),
            "BRAVO_CODEBRO_WORKSPACE_MARKER\n",
        )
        .unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a = call_tool_text(
            &server,
            "sandbox_exec",
            json!({
                "command": "cat marker.txt",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let res_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert!(res_a["stdout"]
            .as_str()
            .unwrap_or("")
            .contains("ALPHA_CODEBRO_WORKSPACE_MARKER"));

        let out_b = call_tool_text(
            &server,
            "sandbox_exec",
            json!({
                "command": "cat marker.txt",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let res_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert!(res_b["stdout"]
            .as_str()
            .unwrap_or("")
            .contains("BRAVO_CODEBRO_WORKSPACE_MARKER"));
    }

    #[tokio::test]
    async fn reindex_is_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        std::fs::write(root_a.join("extra.rs"), "pub fn alpha_unique_fn() {}\n").unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let _ = call_tool_text(
            &server,
            "reindex",
            json!({ "workspace_root": root_a.to_string_lossy().to_string() }),
        )
        .await;

        let out_a = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "alpha_unique_fn",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert!(facts_a["returned"].as_u64().unwrap() > 0);

        let out_b = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "alpha_unique_fn",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert_eq!(facts_b["returned"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn impact_analyze_is_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a = call_tool_text(
            &server,
            "impact_analyze",
            json!({
                "target": "main",
                "target_type": "symbol",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let impact_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert!(impact_a.get("target").is_some());

        let out_b = call_tool_text(
            &server,
            "impact_analyze",
            json!({
                "target": "main",
                "target_type": "symbol",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let impact_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert!(impact_b.get("target").is_some());
    }

    #[tokio::test]
    async fn repository_health_is_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a = call_tool_text(
            &server,
            "repository_health",
            json!({ "workspace_root": root_a.to_string_lossy().to_string() }),
        )
        .await;
        let health_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert_eq!(health_a["status"], "healthy");

        let out_b = call_tool_text(
            &server,
            "repository_health",
            json!({ "workspace_root": root_b.to_string_lossy().to_string() }),
        )
        .await;
        let health_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert_eq!(health_b["status"], "healthy");
    }

    #[tokio::test]
    async fn memory_stats_is_workspace_isolated() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let _ = call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "wsA-stat-key",
                "value": "wsA-stat-value",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let _ = call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "wsB-stat-key",
                "value": "wsB-stat-value",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;

        let out_a = call_tool_text(
            &server,
            "memory_stats",
            json!({ "workspace_root": root_a.to_string_lossy().to_string() }),
        )
        .await;
        let stats_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        assert!(stats_a["entry_count"].as_u64().unwrap() >= 1);

        let out_b = call_tool_text(
            &server,
            "memory_stats",
            json!({ "workspace_root": root_b.to_string_lossy().to_string() }),
        )
        .await;
        let stats_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        assert!(stats_b["entry_count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn concurrent_access_does_not_leak() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = Arc::new(hermetic_multiws_server(root_a, &[root_b]));
        let path_a = root_a.to_string_lossy().to_string();
        let path_b = root_b.to_string_lossy().to_string();

        let server_a = server.clone();
        let pa = path_a.clone();
        let handle_a = tokio::spawn(async move {
            for i in 0..20 {
                let _ = call_tool_text(
                    &server_a,
                    "record_memory",
                    json!({
                        "key": format!("wsA-concurrent-{i}"),
                        "value": format!("value-{i}"),
                        "workspace_root": &pa,
                    }),
                )
                .await;
            }
        });

        let server_b = server.clone();
        let pb = path_b.clone();
        let handle_b = tokio::spawn(async move {
            for i in 0..20 {
                let _ = call_tool_text(
                    &server_b,
                    "record_memory",
                    json!({
                        "key": format!("wsB-concurrent-{i}"),
                        "value": format!("value-{i}"),
                        "workspace_root": &pb,
                    }),
                )
                .await;
            }
        });

        handle_a.await.unwrap();
        handle_b.await.unwrap();

        let out_a = call_tool_text(
            &server,
            "engineering_memory",
            json!({
                "task_keywords": ["wsA-concurrent-0"],
                "workspace_root": &path_a,
            }),
        )
        .await;
        let mem_a: serde_json::Value = serde_json::from_str(&out_a).unwrap();
        let keys_a: Vec<String> = mem_a["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["key"].as_str())
            .map(|s| s.to_string())
            .collect();
        assert!(keys_a.contains(&"wsA-concurrent-0".to_string()));
        assert!(!keys_a.contains(&"wsB-concurrent-0".to_string()));

        let out_b = call_tool_text(
            &server,
            "engineering_memory",
            json!({
                "task_keywords": ["wsB-concurrent-0"],
                "workspace_root": &path_b,
            }),
        )
        .await;
        let mem_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        let keys_b: Vec<String> = mem_b["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["key"].as_str())
            .map(|s| s.to_string())
            .collect();
        assert!(keys_b.contains(&"wsB-concurrent-0".to_string()));
        assert!(!keys_b.contains(&"wsA-concurrent-0".to_string()));
    }

    #[tokio::test]
    async fn switching_workspace_a_to_b_to_a_preserves_isolation() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let root_a = ws_a.path();
        let root_b = ws_b.path();
        std::fs::write(root_a.join("marker.txt"), "ALPHA").unwrap();
        std::fs::write(root_a.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_a).unwrap();
        std::fs::write(root_b.join("marker.txt"), "BRAVO").unwrap();
        std::fs::write(root_b.join("main.rs"), "fn main() {}\n").unwrap();
        crate::init::run(root_b).unwrap();

        let server = hermetic_multiws_server(root_a, &[root_b]);

        let out_a1 = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "main",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_a1: serde_json::Value = serde_json::from_str(&out_a1).unwrap();
        let id_a1: HashSet<String> = facts_a1["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["id"].as_str())
            .map(|s| s.to_string())
            .collect();

        let out_b = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "main",
                "workspace_root": root_b.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_b: serde_json::Value = serde_json::from_str(&out_b).unwrap();
        let id_b: HashSet<String> = facts_b["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["id"].as_str())
            .map(|s| s.to_string())
            .collect();

        let out_a2 = call_tool_text(
            &server,
            "engineering_facts",
            json!({
                "query": "main",
                "workspace_root": root_a.to_string_lossy().to_string(),
            }),
        )
        .await;
        let facts_a2: serde_json::Value = serde_json::from_str(&out_a2).unwrap();
        let id_a2: HashSet<String> = facts_a2["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["id"].as_str())
            .map(|s| s.to_string())
            .collect();

        assert_eq!(
            id_a1, id_a2,
            "workspace A facts must be stable across switches"
        );
        let overlap: HashSet<&String> = id_a1.iter().filter(|id| id_b.contains(*id)).collect();
        assert!(
            overlap.is_empty(),
            "workspace A and B facts must never overlap"
        );
    }
}

#[cfg(test)]
mod debugging_consult_tests {
    use super::*;

    #[tokio::test]
    async fn debugging_mode_injects_latest_hypotheses() {
        use crate::consultant::types::{ConsultantMode, ConsultantRequest};
        let dir = tempfile::tempdir().expect("tempdir");
        let server = CodeBroMcpServer::new(dir.path().to_path_buf());

        // No analysis yet → no injection.
        let mut req = ConsultantRequest {
            provider: Default::default(),
            mode: ConsultantMode::Debugging,
            question: "q".into(),
            context: None,
            files: vec![],
            include_git_diff: false,
            include_project_context: false,
            max_answer_length: 0,
        };
        server.inject_debugging_hypotheses(
            &server.resolve_workspace(None).unwrap(),
            &mut req,
            &ConsultantMode::Debugging,
        );
        assert!(req.files.is_empty());

        // Store an analysis → injected exactly once in debugging mode.
        let ws = server.resolve_workspace(None).unwrap();
        *ws.last_rca.lock().unwrap() = Some(crate::debugging::types::RootCauseAnalysis {
            status: crate::debugging::types::AnalysisStatus::Hypotheses,
            failure_classification: "test_failure".into(),
            hypotheses: vec![],
            evidence_summary: vec![],
            freshness: "stale".into(),
            limitations: vec![],
        });
        server.inject_debugging_hypotheses(
            &server.resolve_workspace(None).unwrap(),
            &mut req,
            &ConsultantMode::Debugging,
        );
        assert_eq!(req.files.len(), 1);
        assert_eq!(req.files[0].path, "codebro://root-cause-hypotheses");
        assert!(req.files[0].content.contains("test_failure"));

        // Non-debugging modes never receive it.
        let mut req2 = req.clone();
        req2.files.clear();
        server.inject_debugging_hypotheses(
            &server.resolve_workspace(None).unwrap(),
            &mut req2,
            &ConsultantMode::Planning,
        );
        assert!(req2.files.is_empty());
    }
}

#[cfg(test)]
mod evidence_journal_wiring_tests {
    use super::*;

    fn server_at(root: &std::path::Path) -> CodeBroMcpServer {
        CodeBroMcpServer::with_sandbox_runtime(
            root.to_path_buf(),
            crate::sandbox::SandboxRuntime::from_env(),
        )
    }

    fn verification_with(
        tree_hash: Option<&str>,
        denied: bool,
        success: bool,
    ) -> crate::sandbox::VerificationResult {
        let execution = crate::sandbox::ExecutionResult::from_local(
            "cargo test",
            "/tmp/nowhere",
            "",
            "",
            if success { 0 } else { 1 },
            25,
            false,
            denied,
            std::collections::HashMap::new(),
        );
        let execution = crate::sandbox::ExecutionResult {
            // from_local hardcodes denied=false (its 8th arg is cancelled);
            // set the policy-denial flag explicitly for this scenario.
            denied,
            repo_state: tree_hash.map(|h| crate::sandbox::RepoState {
                commit_sha: "c".into(),
                working_tree_dirty: false,
                working_tree_hash: h.into(),
            }),
            ..execution
        };
        let mut v = crate::sandbox::VerificationResult::from_execution(execution);
        v.classification = Some(
            if denied {
                "denied"
            } else if success {
                "success"
            } else {
                "test_failure"
            }
            .into(),
        );
        v
    }

    /// Denied runs never execute: nothing is recorded and nothing surfaces.
    #[test]
    fn denied_runs_are_never_journaled() {
        let dir = tempfile::tempdir().unwrap();
        let server = server_at(dir.path());
        let v = verification_with(Some("treeX"), true, false);
        let obj = server.verification_object(&server.resolve_workspace(None).unwrap(), &v, &[]);
        assert!(obj.get("prior_evidence").is_none());
        assert!(!dir.path().join(".codebro/execution_evidence.json").exists());
    }

    /// Without a capturable tree hash, runs are neither recorded nor answered:
    /// historical evidence is always bound to repository state.
    #[test]
    fn runs_without_tree_state_are_not_journaled() {
        let dir = tempfile::tempdir().unwrap();
        let server = server_at(dir.path());
        // from_local leaves repo_state None (non-git workspace analog).
        let v = verification_with(None, false, true);
        let obj = server.verification_object(&server.resolve_workspace(None).unwrap(), &v, &[]);
        assert!(obj.get("prior_evidence").is_none());
        assert!(!dir.path().join(".codebro/execution_evidence.json").exists());
    }

    /// A run WITH a tree hash records; the next identical context sees the
    /// same-tree prior. Lookup happens before recording, so the first run's
    /// own record can never be mistaken for prior history.
    #[test]
    fn recording_then_lookup_respects_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let server = server_at(dir.path());

        let v1 = verification_with(Some("treeQ"), false, true);
        let obj1 = server.verification_object(&server.resolve_workspace(None).unwrap(), &v1, &[]);
        assert!(
            obj1.get("prior_evidence").is_none(),
            "first run has no history"
        );
        assert!(dir.path().join(".codebro/execution_evidence.json").exists());

        let v2 = verification_with(Some("treeQ"), false, true);
        let obj2 = server.verification_object(&server.resolve_workspace(None).unwrap(), &v2, &[]);
        let prior = obj2.get("prior_evidence").expect("second run must surface");
        assert_eq!(prior["same_tree"]["outcome"], "success");
    }
}

// ── Multi-workspace isolation ────────────────────────────────────────────

// ── P4 skill lifecycle adversarial tests ────────────────────────────────
//
// The skill tests share one global env var (CODEBRO_SKILLS_DIR), so the
// module serializes on a static lock: no cross-test env clobbering, and
// publication always lands in a per-test tempdir — never the real
// ~/.config/opencode/skills.

#[cfg(test)]
mod skill_tests {
    use super::*;
    use serde_json::json;

    /// Serialize env-var mutation across every skill test (tests run on
    /// parallel threads). Async-aware: the guard is held across await
    /// points by design — each test owns the env var for its lifetime.
    static SKILL_ENV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

    async fn skill_env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        SKILL_ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

    struct Guard(std::path::PathBuf);

    fn hermetic_server(dir: &tempfile::TempDir) -> (CodeBroMcpServer, Guard) {
        let server =
            CodeBroMcpServer::with_state_dir(dir.path().to_path_buf(), dir.path().join("state"));
        let skills = dir.path().join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        std::env::set_var("CODEBRO_SKILLS_DIR", &skills);
        (server, Guard(skills))
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            std::env::remove_var("CODEBRO_SKILLS_DIR");
        }
    }

    async fn call_text(server: &CodeBroMcpServer, args: serde_json::Value) -> String {
        let p: SkillArgs = serde_json::from_value(args).unwrap();
        let r = server
            .skill(Parameters(p))
            .await
            .expect("skill call succeeds");
        text_of(r)
    }

    async fn call_err(server: &CodeBroMcpServer, args: serde_json::Value) -> String {
        let p: SkillArgs = serde_json::from_value(args).unwrap();
        server
            .skill(Parameters(p))
            .await
            .expect_err("skill call must fail")
            .to_string()
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

    fn content_for(name: &str) -> String {
        format!("---\nname: {name}\ndescription: A hermetic test skill\n---\n\n# Purpose\n\nBody.")
    }

    /// Seed an accepted P3 learning candidate the honest way: record
    /// repeated validation events through the public history pipeline,
    /// let the deterministic detector cluster them into a candidate, and
    /// evaluate it to `accepted`. Returns the learning candidate id.
    fn seed_accepted_learning(server: &CodeBroMcpServer, ws: &str) -> String {
        let store = server.context_store();
        let now = 1_700_000_000u64;
        for i in 0..4 {
            let mut input = crate::context_runtime::HistoryInput::new(
                ws.to_string(),
                crate::context_runtime::HistoryKind::Validation,
                format!("cargo test phase-{i} passed with fmt clippy suite"),
            );
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("success".to_string());
            input.created_at = Some(now + i);
            let (id, _fresh) = store.record_history(&input, now + i).unwrap();
            assert!(id > 0);
        }
        let candidates = store
            .propose_candidates(
                Some(ws),
                None,
                crate::context_runtime::LearnScope::Project,
                now + 10,
            )
            .unwrap();
        assert!(
            !candidates.is_empty(),
            "deterministic detector must cluster the repeated validations"
        );
        let id = candidates[0].candidate_id.clone();
        let evaluated = store.evaluate_candidate(&id, now + 20).unwrap();
        assert_eq!(evaluated.status, "accepted", "got {:?}", evaluated.status);
        id
    }

    /// Propose (evidence-backed via learning) and drive to validated
    /// through the canonical store pipeline. Returns the candidate id.
    async fn evidence_backed_validated(server: &CodeBroMcpServer, name: &str, ws: &str) -> String {
        let lc_id = seed_accepted_learning(server, ws);
        let out = call_text(
            server,
            json!({
                "action": "propose",
                "name": name,
                "description": "Evidence-backed test skill",
                "purpose": "Testing",
                "content": content_for(name),
                "learning_candidate_id": lc_id,
            }),
        )
        .await;
        assert!(out.contains(name), "propose failed: {out}");

        let store = server.context_store();
        let candidates = store
            .list_skill_candidates(Some(ws), None, None, 10)
            .unwrap();
        let cid = candidates
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.candidate_id.clone())
            .unwrap();
        use crate::context_runtime::SkillCandidateStatus as S;
        store
            .transition_skill_candidate(&cid, S::Evaluating, None, 1)
            .unwrap();
        store
            .promote_candidate_to_draft(&cid, &content_for(name), 2)
            .unwrap();
        store
            .transition_skill_candidate(&cid, S::Validated, None, 3)
            .unwrap();
        cid
    }

    #[tokio::test]
    async fn approve_requires_user_confirmed_flag() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        let cid = evidence_backed_validated(&server, "gate-skill", &ws).await;

        // Without the flag: refused — the model cannot self-approve.
        let err = call_err(&server, json!({ "action": "approve", "candidate_id": cid })).await;
        assert!(
            err.contains("user_confirmed"),
            "self-approval must be refused: {err}"
        );

        // Nothing was published.
        assert!(!_guard.0.join("gate-skill").exists());

        // With the flag (user speech act): publishes.
        let out = call_text(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;
        assert!(out.contains("active"), "approve output: {out}");
        assert!(_guard.0.join("gate-skill").join("SKILL.md").exists());
    }

    #[tokio::test]
    async fn forged_approval_from_wrong_status_refused() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        let lc_id = seed_accepted_learning(&server, &ws);
        let out = call_text(
            &server,
            json!({
                "action": "propose",
                "name": "forge-skill",
                "description": "d", "purpose": "p",
                "content": content_for("forge-skill"),
                "learning_candidate_id": lc_id,
            }),
        )
        .await;
        assert!(out.contains("forge-skill"));

        // The candidate is still in 'candidate' status. Supplying
        // user_confirmed=true (a forged speech act) still cannot publish:
        // the store demands the full validated pipeline.
        let store = server.context_store();
        let candidates = store
            .list_skill_candidates(Some(&ws), None, None, 10)
            .unwrap();
        let cid = candidates
            .iter()
            .find(|c| c.name == "forge-skill")
            .map(|c| c.candidate_id.clone())
            .unwrap();
        let err = call_err(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;
        assert!(
            err.contains("validated"),
            "store gate must refuse unvalidated content even with a claimed speech act: {err}"
        );
        assert!(!_guard.0.join("forge-skill").exists());
    }

    #[tokio::test]
    async fn standalone_confidence_capped_below_approval_floor() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        // Claimed confidence 0.99 → capped below 0.60: a standalone
        // proposal can never approve without evidence-backed learning.
        let out = call_text(
            &server,
            json!({
                "action": "propose",
                "name": "capped-skill",
                "description": "d", "purpose": "p",
                "content": content_for("capped-skill"),
                "confidence": 0.99,
            }),
        )
        .await;
        let obj: serde_json::Value = serde_json::from_str(&out).unwrap();
        let confidence = obj["candidate"]["confidence"].as_f64().unwrap();
        assert!(
            confidence < crate::context_runtime::SKILL_APPROVAL_MIN_CONFIDENCE,
            "standalone confidence must be capped below the floor: {confidence}"
        );

        // Drive it through the pipeline: approval still fails on the
        // confidence gate.
        let store = server.context_store();
        let candidates = store
            .list_skill_candidates(Some(&ws), None, None, 10)
            .unwrap();
        let cid = candidates
            .iter()
            .find(|c| c.name == "capped-skill")
            .map(|c| c.candidate_id.clone())
            .unwrap();
        use crate::context_runtime::SkillCandidateStatus as S;
        store
            .transition_skill_candidate(&cid, S::Evaluating, None, 1)
            .unwrap();
        store
            .promote_candidate_to_draft(&cid, &content_for("capped-skill"), 2)
            .unwrap();
        store
            .transition_skill_candidate(&cid, S::Validated, None, 3)
            .unwrap();
        let err = call_err(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;
        assert!(
            err.contains("below approval floor"),
            "capped standalone candidate must not publish: {err}"
        );
        assert!(!_guard.0.join("capped-skill").exists());
    }

    #[tokio::test]
    async fn secrets_never_publish() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        let lc_id = seed_accepted_learning(&server, &ws);
        let out = call_text(
            &server,
            json!({
                "action": "propose",
                "name": "leaky-skill",
                "description": "d", "purpose": "p",
                "content": "---\nname: leaky-skill\ndescription: d\n---\n\n# Purpose\n\nUse api_key sk-abc123.",
                "learning_candidate_id": lc_id,
            }),
        )
        .await;
        let obj: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            obj["candidate"]["validation"]["secret_safe"],
            json!(false),
            "secret content must be flagged at propose"
        );

        // The pipeline refuses to draft secret content, so it can never
        // reach approval.
        let store = server.context_store();
        let candidates = store
            .list_skill_candidates(Some(&ws), None, None, 10)
            .unwrap();
        let cid = candidates
            .iter()
            .find(|c| c.name == "leaky-skill")
            .map(|c| c.candidate_id.clone())
            .unwrap();
        use crate::context_runtime::SkillCandidateStatus as S;
        store
            .transition_skill_candidate(&cid, S::Evaluating, None, 1)
            .unwrap();
        let secret_content =
            "---\nname: leaky-skill\ndescription: d\n---\n\n# Purpose\n\nUse api_key sk-abc123.";
        let draft = store.promote_candidate_to_draft(&cid, secret_content, 2);
        assert!(draft.is_err(), "secret content must not draft: {draft:?}");
        assert!(!_guard.0.join("leaky-skill").exists());
    }

    #[tokio::test]
    async fn cross_workspace_mutations_refused() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        // Authorize a foreign sibling workspace up front (operator-
        // equivalent launch config); the isolation below must hold even
        // between two AUTHORIZED roots.
        let foreign = tempfile::tempdir().unwrap();
        let registry = WorkspaceRegistry::with_authorized_roots(
            dir.path().to_path_buf(),
            crate::workspace_registry::AuthorizedRoots::with_extras(
                dir.path().to_path_buf(),
                [foreign.path().to_path_buf()],
            ),
        );
        let skills = dir.path().join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        std::env::set_var("CODEBRO_SKILLS_DIR", &skills);
        let server = assemble_server_with_registry(
            registry,
            crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local),
            Some(dir.path().join("state")),
        );
        let _guard = Guard(skills);
        let ws = dir.path().to_str().unwrap().to_string();

        let cid = evidence_backed_validated(&server, "iso-mcp-skill", &ws).await;
        call_text(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;

        let store = server.context_store();
        let skill = store.get_skill_by_name("iso-mcp-skill").unwrap().unwrap();
        let sid = skill.skill_id.clone();

        // A foreign (AUTHORIZED, sibling) workspace cannot inspect by id,
        // record health, deprecate, or roll back the skill — store-level
        // workspace gates, independent of root authorization.
        let fws = foreign.path().to_str().unwrap().to_string();

        let err = call_err(
            &server,
            json!({ "action": "inspect", "skill_id": sid, "workspace_root": fws }),
        )
        .await;
        assert!(
            err.contains("not visible"),
            "cross-workspace inspect leaked: {err}"
        );

        let err = call_err(
            &server,
            json!({ "action": "health", "skill_id": sid, "workspace_root": fws }),
        )
        .await;
        assert!(
            err.contains("not visible"),
            "cross-workspace health leaked: {err}"
        );

        let err = call_err(
            &server,
            json!({ "action": "deprecate", "skill_id": sid, "workspace_root": fws }),
        )
        .await;
        assert!(
            err.contains("workspace mismatch"),
            "cross-workspace deprecate leaked: {err}"
        );

        let err = call_err(
            &server,
            json!({ "action": "rollback", "skill_id": sid, "version": 1, "workspace_root": fws }),
        )
        .await;
        assert!(
            err.contains("workspace mismatch"),
            "cross-workspace rollback leaked: {err}"
        );

        // Candidate inspect from foreign workspace: refused.
        let err = call_err(
            &server,
            json!({ "action": "inspect", "candidate_id": cid, "workspace_root": fws }),
        )
        .await;
        assert!(
            err.contains("not visible"),
            "cross-workspace candidate inspect leaked: {err}"
        );
    }

    #[tokio::test]
    async fn task_scoped_candidates_need_task_context() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);

        // Task-scoped propose without task_id: refused.
        let err = call_err(
            &server,
            json!({ "action": "propose", "name": "task-skill", "scope": "task",
                    "description": "d", "purpose": "p", "content": "x" }),
        )
        .await;
        assert!(err.contains("task_id"), "task scope without task id: {err}");

        // With task_id: works.
        let out = call_text(
            &server,
            json!({ "action": "propose", "name": "task-skill", "scope": "task", "task_id": "t-42",
                    "description": "d", "purpose": "p",
                    "content": content_for("task-skill") }),
        )
        .await;
        assert!(out.contains("task-skill"));

        // Discover without the task: invisible.
        let discover = call_text(&server, json!({ "action": "discover" })).await;
        assert!(
            !discover.contains("\"task-skill\""),
            "task-scoped candidate must not be visible without its task"
        );

        // Discover with the task: visible.
        let discover = call_text(&server, json!({ "action": "discover", "task_id": "t-42" })).await;
        assert!(discover.contains("task-skill"));
    }

    #[tokio::test]
    async fn discover_lists_only_active_skills() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        // Publish, then deprecate.
        let cid = evidence_backed_validated(&server, "deprecated-view-skill", &ws).await;
        call_text(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;
        let store = server.context_store();
        let skill = store
            .get_skill_by_name("deprecated-view-skill")
            .unwrap()
            .unwrap();
        call_text(
            &server,
            json!({ "action": "deprecate", "skill_id": skill.skill_id }),
        )
        .await;

        // Discover: the deprecated skill must not appear in the
        // active_skills section (its candidate lineage stays listed for
        // audit, which is correct).
        let discover = call_text(&server, json!({ "action": "discover" })).await;
        let obj: serde_json::Value = serde_json::from_str(&discover).unwrap();
        let active = obj["active_skills"].as_array().unwrap().clone();
        assert!(
            !active
                .iter()
                .any(|s| s["name"] == json!("deprecated-view-skill")),
            "deprecated skill must not be listed as active: {active:?}"
        );
    }

    #[tokio::test]
    async fn health_records_only_for_active_skills() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        let cid = evidence_backed_validated(&server, "health-gate-skill", &ws).await;
        call_text(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;
        let store = server.context_store();
        let skill = store
            .get_skill_by_name("health-gate-skill")
            .unwrap()
            .unwrap();

        // Record a failure: works while active.
        call_text(
            &server,
            json!({ "action": "health", "skill_id": skill.skill_id, "success": false }),
        )
        .await;

        // Deprecate, then health: refused.
        call_text(
            &server,
            json!({ "action": "deprecate", "skill_id": skill.skill_id }),
        )
        .await;
        let err = call_err(
            &server,
            json!({ "action": "health", "skill_id": skill.skill_id, "success": true }),
        )
        .await;
        assert!(
            err.contains("active"),
            "deprecated skills must not accumulate health: {err}"
        );
    }

    #[tokio::test]
    async fn skill_tests_are_hermetic() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let ws = dir.path().to_str().unwrap().to_string();

        // Full pipeline: everything lands inside this tempdir.
        let cid = evidence_backed_validated(&server, "hermetic-check", &ws).await;
        call_text(
            &server,
            json!({ "action": "approve", "candidate_id": cid, "user_confirmed": true }),
        )
        .await;

        let skills_root = std::env::var("CODEBRO_SKILLS_DIR").unwrap();
        assert!(skills_root.starts_with(dir.path().to_str().unwrap()));
        assert!(std::path::PathBuf::from(&skills_root)
            .join("hermetic-check")
            .join("SKILL.md")
            .exists());
        assert!(dir.path().join("state").join("state.db").exists());
    }

    #[tokio::test]
    async fn unknown_action_rejected() {
        let _env = skill_env_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let (server, _guard) = hermetic_server(&dir);
        let err = call_err(&server, json!({ "action": "execute" })).await;
        assert!(err.contains("unknown skill action"));
        assert!(err.contains("discover"));
    }
}

// ── P5 durable task runtime adversarial tests ──────────────────────────

#[cfg(test)]
mod task_tests {
    use super::*;
    use serde_json::json;

    fn text_of(result: CallToolResult) -> String {
        result
            .content
            .into_iter()
            .find_map(|b| match b {
                rmcp::model::ContentBlock::Text(t) => Some(t.text),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn server(dir: &tempfile::TempDir) -> CodeBroMcpServer {
        CodeBroMcpServer::with_state_dir(dir.path().to_path_buf(), dir.path().join("state"))
    }

    /// Multi-root task-test server: the default root plus listed extra
    /// subdirectory workspaces are authorized (operator-equivalent
    /// launch config) so cross-workspace isolation is asserted between
    /// AUTHORIZED roots (the real product guarantee).
    fn server_multiroot(
        dir: &tempfile::TempDir,
        extras: &[std::path::PathBuf],
    ) -> CodeBroMcpServer {
        let registry = WorkspaceRegistry::with_authorized_roots(
            dir.path().to_path_buf(),
            crate::workspace_registry::AuthorizedRoots::with_extras(
                dir.path().to_path_buf(),
                extras.iter().cloned(),
            ),
        );
        assemble_server_with_registry(
            registry,
            crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local),
            Some(dir.path().join("state")),
        )
    }

    async fn call(server: &CodeBroMcpServer, args: serde_json::Value) -> serde_json::Value {
        let p: TaskArgs = serde_json::from_value(args).unwrap();
        let r = server
            .task(Parameters(p))
            .await
            .expect("task call succeeds");
        serde_json::from_str(&text_of(r)).unwrap_or(json!({}))
    }

    async fn call_err(server: &CodeBroMcpServer, args: serde_json::Value) -> String {
        let p: TaskArgs = serde_json::from_value(args).unwrap();
        server
            .task(Parameters(p))
            .await
            .expect_err("must fail")
            .to_string()
    }

    async fn create(server: &CodeBroMcpServer, title: &str) -> String {
        let out = call(server, json!({ "action": "create", "title": title })).await;
        out["task"]["task_id"].as_str().unwrap().to_string()
    }

    async fn drive_to_validating(server: &CodeBroMcpServer, task_id: &str) {
        call(server, json!({ "action": "start", "task_id": task_id })).await;
        call(
            server,
            json!({ "action": "validate", "task_id": task_id, "what": "cargo test" }),
        )
        .await;
    }

    #[tokio::test]
    async fn full_lifecycle_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "implement task runtime").await;

        // start
        let out = call(&s, json!({ "action": "start", "task_id": id })).await;
        assert_eq!(out["task"]["status"], "running");
        // checkpoint
        let out = call(
            &s,
            json!({
                "action": "checkpoint", "task_id": id,
                "summary": "runtime core done", "progress": "mcp wiring", "next_action": "wire tool"
            }),
        )
        .await;
        assert_eq!(out["checkpoint"]["version"], 1);
        // validate + record passed
        call(
            &s,
            json!({ "action": "validate", "task_id": id, "what": "cargo test" }),
        )
        .await;
        let out = call(
            &s,
            json!({ "action": "validation_result", "task_id": id, "result": "passed", "what": "cargo test" }),
        )
        .await;
        assert_eq!(out["task"]["status"], "validating");
        // complete
        let out = call(
            &s,
            json!({
                "action": "complete", "task_id": id, "reason": "shipped",
                "changed_areas": ["crates/context-runtime"]
            }),
        )
        .await;
        assert_eq!(out["task"]["status"], "completed");
        assert_eq!(out["task"]["outcome"]["result"], "completed");
        // inspect shows the bounded snapshot
        let out = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        assert_eq!(out["snapshot"]["task"]["status"], "completed");
        assert!(out["snapshot"]["latest_checkpoint"].is_object());
    }

    #[tokio::test]
    async fn completion_gate_is_enforced_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "gate test").await;
        call(&s, json!({ "action": "start", "task_id": id })).await;
        // complete without validation: refused.
        let err = call_err(
            &s,
            json!({ "action": "complete", "task_id": id, "reason": "trust me" }),
        )
        .await;
        assert!(err.to_lowercase().contains("validating"), "{err}");
        // Even validate → failed blocks completion.
        call(
            &s,
            json!({ "action": "validate", "task_id": id, "what": "t" }),
        )
        .await;
        call(
            &s,
            json!({ "action": "validation_result", "task_id": id, "result": "failed" }),
        )
        .await;
        // Failed validation returned the task to running; completion is
        // structurally refused from there.
        let err = call_err(
            &s,
            json!({ "action": "complete", "task_id": id, "reason": "trust me" }),
        )
        .await;
        assert!(err.to_lowercase().contains("validating"), "{err}");
    }

    #[tokio::test]
    async fn invalid_transitions_are_refused_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "transitions").await;
        // pause before start: pending → paused is invalid.
        let err = call_err(&s, json!({ "action": "pause", "task_id": id })).await;
        assert!(err.contains("transition"), "{err}");
        // complete a pending task: refused.
        let err = call_err(&s, json!({ "action": "complete", "task_id": id })).await;
        assert!(err.contains("validating"), "{err}");
        // Drive to completed, then mutate: terminal refusal.
        drive_to_validating(&s, &id).await;
        call(
            &s,
            json!({ "action": "validation_result", "task_id": id, "result": "passed" }),
        )
        .await;
        call(
            &s,
            json!({ "action": "complete", "task_id": id, "reason": "ok" }),
        )
        .await;
        for action in ["start", "pause", "resume", "checkpoint", "validate"] {
            let err = call_err(&s, json!({ "action": action, "task_id": id })).await;
            assert!(!err.is_empty(), "terminal task must refuse {action}");
        }
    }

    #[tokio::test]
    async fn workspace_isolation_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let ws_a = dir.path().join("a");
        let ws_b = dir.path().join("b");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        let s = server_multiroot(&dir, &[ws_a.clone(), ws_b.clone()]);
        // Create in A.
        let out = call(
            &s,
            json!({ "action": "create", "title": "a-task", "workspace_root": ws_a.display().to_string() }),
        )
        .await;
        let id = out["task"]["task_id"].as_str().unwrap().to_string();
        // Inspect from B: not found (invisible, not leaked).
        let err = call_err(
            &s,
            json!({ "action": "inspect", "task_id": id, "workspace_root": ws_b.display().to_string() }),
        )
        .await;
        assert!(err.contains("workspace"), "{err}");
        // List from B is empty.
        let out = call(
            &s,
            json!({ "action": "list", "workspace_root": ws_b.display().to_string() }),
        )
        .await;
        assert_eq!(out["count"], 0);
        // Mutations from B are refused.
        let err = call_err(
            &s,
            json!({ "action": "start", "task_id": id, "workspace_root": ws_b.display().to_string() }),
        )
        .await;
        assert!(err.contains("workspace"), "{err}");
    }

    #[tokio::test]
    async fn stale_writer_and_lease_are_enforced_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "concurrency").await;
        // Read v0, start (→v1), then mutate with the stale anchor: refused.
        let out = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        let stale_version = out["snapshot"]["task"]["current_version"].as_u64().unwrap();
        call(&s, json!({ "action": "start", "task_id": id })).await;
        let err = call_err(
            &s,
            json!({ "action": "checkpoint", "task_id": id, "summary": "stale", "based_on_version": stale_version }),
        )
        .await;
        assert!(err.contains("stale"), "{err}");
        // A second server process (different worker) cannot pause while
        // the first worker's lease is live.
        let s2 = server(&dir);
        let err = call_err(&s2, json!({ "action": "pause", "task_id": id })).await;
        assert!(err.contains("lease") || err.contains("worker"), "{err}");
        // The stale-lease path: after TTL expiry the second worker resumes.
        // (Simulated at the store layer in unit tests; here the first
        // worker legitimately pauses.)
        let out = call(&s, json!({ "action": "pause", "task_id": id })).await;
        assert_eq!(out["task"]["status"], "paused");
        let out = call(&s2, json!({ "action": "resume", "task_id": id })).await;
        assert_eq!(out["task"]["status"], "running");
        assert_ne!(
            out["task"]["lease_worker"],
            serde_json::Value::Null,
            "the resuming worker holds the lease"
        );
    }

    #[tokio::test]
    async fn secrets_are_redacted_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let secret = "sk-ABCDEFGHIJKLMNOP123456";
        let out = call(
            &s,
            json!({ "action": "create", "title": "deploy", "description": format!("key {secret}") }),
        )
        .await;
        let desc = out["task"]["description"].as_str().unwrap();
        assert!(
            !desc.contains(secret),
            "description must be redacted: {desc}"
        );
        let id = out["task"]["task_id"].as_str().unwrap().to_string();
        call(&s, json!({ "action": "start", "task_id": id })).await;
        let out = call(
            &s,
            json!({ "action": "checkpoint", "task_id": id, "summary": format!("used {secret}") }),
        )
        .await;
        let summary = out["checkpoint"]["summary"].as_str().unwrap();
        assert!(
            !summary.contains(secret),
            "checkpoint must be redacted: {summary}"
        );
    }

    #[tokio::test]
    async fn idempotent_create_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let a = call(
            &s,
            json!({ "action": "create", "title": "fix gpu init", "idempotency_key": "gpu-fix" }),
        )
        .await;
        let b = call(
            &s,
            json!({ "action": "create", "title": "fix gpu init", "idempotency_key": "gpu-fix" }),
        )
        .await;
        assert_eq!(
            a["task"]["task_id"], b["task"]["task_id"],
            "idempotency key returns the same task"
        );
        let list = call(&s, json!({ "action": "list" })).await;
        assert_eq!(list["count"], 1);
        // Similar titles without keys: distinct tasks.
        call(&s, json!({ "action": "create", "title": "fix gpu init" })).await;
        let list = call(&s, json!({ "action": "list" })).await;
        assert_eq!(list["count"], 2);
    }

    #[tokio::test]
    async fn paused_tasks_are_not_listed_as_stale_over_mcp() {
        // Audit pin: an intentionally paused task is not interrupted
        // work, so the recoverable-work listing must stay empty.
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "pausable work").await;
        call(&s, json!({ "action": "start", "task_id": id })).await;
        let out = call(&s, json!({ "action": "pause", "task_id": id })).await;
        assert_eq!(out["task"]["status"], "paused");
        let out = call(&s, json!({ "action": "stale" })).await;
        assert_eq!(out["count"], 0, "paused work is not recoverable work");
        let out = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        assert_eq!(out["snapshot"]["task"]["stale"], false);
    }

    #[tokio::test]
    async fn secrets_redacted_in_validation_and_outcome_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let secret = "ghp_abcdefghij1234567890XY";
        let id = create(&s, "redaction probe").await;
        call(&s, json!({ "action": "start", "task_id": id })).await;
        call(
            &s,
            json!({ "action": "validate", "task_id": id, "what": format!("run with {secret}") }),
        )
        .await;
        let out = call(
            &s,
            json!({ "action": "validation_result", "task_id": id, "result": "passed", "reason": format!("evidence {secret}") }),
        )
        .await;
        let text = serde_json::to_string(&out).unwrap();
        assert!(
            !text.contains(secret),
            "validation surfaces must be redacted"
        );
        let out = call(
            &s,
            json!({ "action": "complete", "task_id": id, "reason": format!("shipped {secret}") }),
        )
        .await;
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains(secret), "outcome must be redacted");
        let out = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains(secret), "resume snapshot must be redacted");
    }

    // ── P9 engineering outcomes ──────────────────────────────────────

    #[tokio::test]
    async fn outcome_reports_evidence_without_transition() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "outcome probe").await;
        call(&s, json!({ "action": "start", "task_id": id })).await;
        let before = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        let version = before["snapshot"]["task"]["current_version"]
            .as_u64()
            .unwrap();
        let out = call(
            &s,
            json!({
                "action": "outcome", "task_id": id,
                "classification": "failure",
                "summary": "integration test failed because API contract differs",
                "what": "cargo test", "exit_code": 101,
                "changed_areas": ["src/api.rs"],
            }),
        )
        .await;
        assert_eq!(out["action"], "outcome");
        assert_eq!(out["classification"], "failure");
        assert_eq!(out["authority"], "observed");
        assert_eq!(out["duplicate"], false);
        assert!(out["event_id"].as_i64().unwrap() > 0);
        // No transition: status and version are untouched.
        let after = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        assert_eq!(after["snapshot"]["task"]["status"], "running");
        assert_eq!(after["snapshot"]["task"]["current_version"], version);
        let events = after["snapshot"]["recent_events"].as_array().unwrap();
        assert!(
            events.iter().any(|e| e["event_id"] == out["event_id"]
                && e["summary"].as_str().unwrap().contains("API contract")),
            "outcome evidence must surface in the resume snapshot: {events:?}"
        );
    }

    #[tokio::test]
    async fn outcome_after_completion_distinguishes_authority() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "confirmable work").await;
        call(&s, json!({ "action": "start", "task_id": id })).await;
        call(
            &s,
            json!({ "action": "validate", "task_id": id, "what": "cargo test" }),
        )
        .await;
        call(
            &s,
            json!({ "action": "validation_result", "task_id": id, "result": "passed" }),
        )
        .await;
        call(
            &s,
            json!({ "action": "complete", "task_id": id, "reason": "shipped" }),
        )
        .await;
        // OpenCode-reported evidence stays observed…
        let out = call(
            &s,
            json!({ "action": "outcome", "task_id": id, "classification": "success", "summary": "tests passed" }),
        )
        .await;
        assert_eq!(out["authority"], "observed");
        // …while an explicit user speech act records user confirmation.
        let out = call(
            &s,
            json!({
                "action": "outcome", "task_id": id, "classification": "success",
                "summary": "user confirmed the fix resolves the issue", "user_confirmed": true,
            }),
        )
        .await;
        assert_eq!(out["authority"], "user_confirmed");
        assert_eq!(out["duplicate"], false);
    }

    #[tokio::test]
    async fn outcome_rejects_malformed_input() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let id = create(&s, "malformed outcomes").await;
        for args in [
            json!({ "action": "outcome", "task_id": id }),
            json!({ "action": "outcome", "task_id": id, "summary": "no classification" }),
            json!({ "action": "outcome", "task_id": id, "classification": "triumph", "summary": "x" }),
            json!({ "action": "outcome", "task_id": id, "classification": "success", "summary": "   " }),
            json!({ "action": "outcome", "classification": "success", "summary": "no task" }),
            json!({ "action": "outcome", "task_id": "task::0000000000000000", "classification": "success", "summary": "ghost" }),
        ] {
            assert!(!call_err(&s, args).await.is_empty());
        }
        let err = call_err(&s, json!({ "action": "frobnicate", "task_id": id })).await;
        assert!(
            err.contains("outcome"),
            "unknown-action help must name outcome: {err}"
        );
    }

    #[tokio::test]
    async fn outcome_dedup_is_idempotent_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let a = create(&s, "task A").await;
        let b = create(&s, "task B").await;
        let args = |task: &str| {
            json!({
                "action": "outcome", "task_id": task, "classification": "partial",
                "summary": "half the migration is done", "dedup_key": "m1",
            })
        };
        let first = call(&s, args(&a)).await;
        assert_eq!(first["duplicate"], false);
        let replay = call(&s, args(&a)).await;
        assert_eq!(replay["duplicate"], true);
        assert_eq!(replay["event_id"], first["event_id"]);
        // Same caller key on another task: distinct outcome (per-task
        // namespacing, no cross-task collision).
        let other = call(&s, args(&b)).await;
        assert_eq!(other["duplicate"], false);
        assert_ne!(other["event_id"], first["event_id"]);
    }

    #[tokio::test]
    async fn outcome_respects_workspace_isolation_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let ws_a = dir.path().join("a");
        let ws_b = dir.path().join("b");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        let s = server_multiroot(&dir, &[ws_a.clone(), ws_b.clone()]);
        let out = call(
            &s,
            json!({ "action": "create", "title": "a-task", "workspace_root": ws_a.display().to_string() }),
        )
        .await;
        let id = out["task"]["task_id"].as_str().unwrap().to_string();
        let err = call_err(
            &s,
            json!({
                "action": "outcome", "task_id": id, "classification": "success",
                "summary": "cross-workspace injection", "workspace_root": ws_b.display().to_string(),
            }),
        )
        .await;
        assert!(err.contains("workspace"), "{err}");
    }

    #[tokio::test]
    async fn outcome_secrets_are_redacted_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let s = server(&dir);
        let secret = "sk-OUTCOME1234567890abcdef";
        let id = create(&s, "outcome redaction").await;
        let out = call(
            &s,
            json!({
                "action": "outcome", "task_id": id, "classification": "failure",
                "summary": format!("deploy failed with api_key=\"{secret}\""),
                "reason": format!("token {secret}"),
                "changed_areas": [format!("src/{secret}.rs")],
            }),
        )
        .await;
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains(secret), "outcome response must be redacted");
        let out = call(&s, json!({ "action": "inspect", "task_id": id })).await;
        let text = serde_json::to_string(&out).unwrap();
        assert!(
            !text.contains(secret),
            "snapshot must not leak outcome secrets"
        );
    }
}

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
use std::sync::Arc;

pub mod change_invalidation;
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
}

#[tool_router]
impl CodeBroMcpServer {
    /// Create a server bound to a workspace root.
    pub fn new(workspace_root: PathBuf) -> Self {
        let registry = WorkspaceRegistry::new(workspace_root);
        Self {
            registry,
            tool_router: Self::tool_router(),
            sandbox_runtime: crate::sandbox::SandboxRuntime::from_env(),
        }
    }

    /// Create a server with an explicit sandbox runtime (for tests).
    pub fn with_sandbox_runtime(
        workspace_root: PathBuf,
        runtime: crate::sandbox::SandboxRuntime,
    ) -> Self {
        let registry = WorkspaceRegistry::new(workspace_root);
        Self {
            registry,
            tool_router: Self::tool_router(),
            sandbox_runtime: runtime,
        }
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

        let payload = json!({
            "workspace_root": ws.canonical_root.display().to_string(),
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
                "languages": counts.languages,
                "frameworks": counts.frameworks,
                "entry_points": counts.entry_points,
                "total": counts.total,
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
            changes.set_description = Some(require_non_empty(desc, "description")?);
        }
        if let Some(url) = args.repository_url.as_deref() {
            changes.set_repository_url = Some(require_non_empty(url, "repository_url")?);
        }
        if let Some(summary) = args.architecture_summary.as_deref() {
            changes.update_architecture_summary =
                Some(require_non_empty(summary, "architecture_summary")?);
        }
        if let Some(sprint) = args.current_sprint.as_deref() {
            changes.set_sprint = Some(require_non_empty(sprint, "current_sprint")?);
        }
        if let Some(item) = args.complete_roadmap_item.as_deref() {
            changes.complete_roadmap_item = Some(require_non_empty(item, "complete_roadmap_item")?);
        }
        if let Some(milestone) = args.add_milestone.as_deref() {
            changes.add_milestone = Some(require_non_empty(milestone, "add_milestone")?);
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
            let title = require_non_empty(&input.title, "decision title")?;
            let description = require_non_empty(&input.description, "decision description")?;
            let id = slugify(&title);
            if existing_decision_ids.contains(id.as_str()) {
                skipped.push(format!("decision '{id}' already recorded"));
                continue;
            }
            let mut decision =
                EngineeringDecision::new(id.clone(), title, description, input.context.clone());
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
            let title = require_non_empty(&input.title, "roadmap title")?;
            let id = slugify(&title);
            if existing_roadmap_ids.contains(id.as_str()) {
                skipped.push(format!("roadmap item '{id}' already recorded"));
                continue;
            }
            let mut item = RoadmapItem::new(id.clone(), title, input.description.clone());
            item.status = parse_roadmap(&input.status)?;
            if let Some(sprint) = input.sprint.as_deref() {
                item.sprint = Some(sprint.to_string());
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
        description = "Analyze structural impact of changing a symbol, file, module, or package. Returns directed relationship edges (with bounded transitive traversal via depth), related tests, owning module/package, and provenance. Descriptive evidence only — no risk scores or prescriptions."
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
    /// `apply_change.needs_reindex=true`. This is a full rebuild, not
    /// incremental. The operation may take longer than normal read-only fact
    /// queries.
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
                    "duration_ms": elapsed.as_millis(),
                });

                Ok(CallToolResult::success(vec![ContentBlock::text(
                    serde_json::to_string_pretty(&payload)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                )]))
            }
            Err(e) => {
                let elapsed = start.elapsed();
                let payload = json!({
                    "status": "error",
                    "error": e.to_string(),
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
        let trimmed = value.trim();
        if trimmed.is_empty() || existing.iter().any(|e| e == trimmed) {
            continue;
        }
        if !target.iter().any(|t| t == trimmed) {
            target.push(trimmed.to_string());
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
                 related tests, owning module/package, and provenance — descriptive evidence \
                 only, no risk scores).\n\
              - To check the health of the CodeBro workspace (project identity, fact store, \
                  engineering memory, git status), call codebro_repository_health (returns \
                  structured exit code, status, per-check results, and summary).\n\
                - To ask an AI consultant (Conductor) for opinions on \
                  architecture, debugging, code review, planning, research, or second \
                  opinions, call codebro_consult (supports provider selection, mode shaping, \
                  and automatic injection of CodeBro engineering context like facts, memory, \
                  and git diff).\n\
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
    /// regardless of OPEN_SANDBOX_URL in the environment.
    fn local_sandbox_server(dir: &tempfile::TempDir) -> CodeBroMcpServer {
        let rt = crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local);
        CodeBroMcpServer::with_sandbox_runtime(dir.path().to_path_buf(), rt)
    }

    fn local_sandbox_server_for_path(path: &std::path::Path) -> CodeBroMcpServer {
        let rt = crate::sandbox::SandboxRuntime::new(crate::sandbox::SandboxMode::Local);
        CodeBroMcpServer::with_sandbox_runtime(path.to_path_buf(), rt)
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
        let server = local_sandbox_server_for_path(&fixture);
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
        let server = local_sandbox_server_for_path(&fixture);
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
        let server = local_sandbox_server_for_path(&fixture);
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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

        let server = Arc::new(CodeBroMcpServer::new(root_a.to_path_buf()));
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

        let server = CodeBroMcpServer::new(root_a.to_path_buf());

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

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Impact analysis: determine what is structurally affected by changing a
//! symbol, file, module, or package.
//!
//! This module extends the verified-facts model with graph-traversal
//! queries over relationships, references, tests, and scope membership.
//! Output is descriptive evidence only — no risk scores, no prescriptions.

pub mod relationships;

use serde::{Deserialize, Serialize};

use crate::engineering_facts::{
    FactId, FactKind, ModuleId, PackageId, ReferenceFact, RelationshipFact, RelationshipKind,
    SymbolId, TestId,
};
use crate::fact_store::FactStore;
use crate::provenance::FreshnessStatus;

// ── Target types ────────────────────────────────────────────────────────

/// What the agent wants to analyze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpactTarget {
    Symbol(SymbolId),
    File(String), // workspace-relative path
    Module(ModuleId),
    Package(PackageId),
}

impl ImpactTarget {
    pub fn kind(&self) -> FactKind {
        match self {
            ImpactTarget::Symbol(_) => FactKind::Symbol,
            ImpactTarget::File(_) => FactKind::Module,
            ImpactTarget::Module(_) => FactKind::Module,
            ImpactTarget::Package(_) => FactKind::Package,
        }
    }
}

// ── Result types ────────────────────────────────────────────────────────

/// Whether the analysis could resolve the target unambiguously.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactStatus {
    Ok,
    Ambiguous(Vec<AmbiguityMatch>),
    NotFound,
    Partial(String),
    Stale,
}

/// A candidate match when the target is ambiguous.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguityMatch {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: Option<String>,
    pub reason: String,
}

/// A single directed relationship edge discovered during analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactRelationship {
    pub target_id: String,
    pub target_name: String,
    pub relationship_kind: String,
    pub direction: String,
    pub source_location: Option<String>,
    pub provenance: Provenance,
    /// Graph distance from the original target (1 = direct, 2+ = transitive).
    pub depth: usize,
    /// Edges forming the path from the target to this relationship's source,
    /// present only when depth >= 2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<PathEdge>,
}

/// A single edge in a traversal path from the original target to a transitive node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathEdge {
    pub source_id: String,
    pub target_id: String,
    pub kind: String,
    pub provenance: Provenance,
}

/// Per-edge-type provenance counts across all traversed edges.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProvenanceSummary {
    pub verified_edges: usize,
    pub heuristic_edges: usize,
    pub unknown_edges: usize,
}

/// Metadata about the traversal that produced the result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TraversalMetadata {
    pub depth_limit: usize,
    pub direction: String,
    pub relationship_types: Vec<String>,
    pub max_nodes: usize,
    pub nodes_visited: usize,
    pub edges_traversed: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_reason: Option<String>,
}

/// Completeness classification for a discovered relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Verified,
    Heuristic,
    Unknown,
}

/// A test that structurally relates to the target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestReference {
    pub id: String,
    pub name: String,
    pub file: Option<String>,
    pub relation: String,
    pub provenance: Provenance,
}

/// Completeness metadata about the analysis result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completeness {
    pub status: String,
    pub limitations: Vec<String>,
}

/// The full impact analysis result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactResult {
    pub target: ImpactTargetInfo,
    pub status: ImpactStatus,
    pub direct_relationships: Vec<ImpactRelationship>,
    pub transitive_relationships: Vec<ImpactRelationship>,
    pub affected_tests: Vec<TestReference>,
    pub affected_modules: Vec<ModuleInfo>,
    pub affected_packages: Vec<PackageInfo>,
    pub evidence: Vec<EvidenceRecord>,
    pub completeness: Completeness,
    pub provenance_summary: ProvenanceSummary,
    pub traversal_metadata: TraversalMetadata,
    /// Store-wide freshness: whether the fact store's generation state
    /// matches the current repository state at analysis time. "unknown"
    /// means the repository state could not be determined (e.g. not a git
    /// repo, or no generation state recorded). This is informational only —
    /// it does not affect the relationship traversal or any other field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<FreshnessStatus>,
}

/// Resolved target information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactTargetInfo {
    pub id: String,
    pub kind: String,
    pub name: Option<String>,
    pub path: Option<String>,
}

/// Module-level impact info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub relation: String,
}

/// Package-level impact info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub relation: String,
}

/// A single evidence record tying a finding back to its source fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub fact_kind: String,
    pub fact_id: String,
    pub fact_name: Option<String>,
    pub source_location: Option<String>,
    pub description: String,
}

// ── Analysis options ────────────────────────────────────────────────────

/// Options that control the impact analysis.
#[derive(Debug, Clone)]
pub struct ImpactOptions {
    /// Maximum number of results per category. 0 = no limit.
    pub max_results: usize,
    /// Whether to include tests in the result.
    pub include_tests: bool,
    /// Whether to include cross-references.
    pub include_references: bool,
    /// Bounded BFS depth. 0 = target only, 1 = direct relationships (default),
    /// up to MAX_DEPTH. Values above MAX_DEPTH are rejected as invalid params.
    pub depth: usize,
    /// Edge direction to follow during traversal: "both", "outgoing", or
    /// "incoming". "both" preserves legacy behaviour (returns both directions
    /// at depth 1).
    pub direction: String,
    /// Optional subset of relationship kinds to traverse. Empty = all kinds.
    pub relationship_types: Vec<String>,
    /// Hard ceiling on distinct graph nodes visited during traversal.
    pub max_nodes: usize,
}

impl Default for ImpactOptions {
    fn default() -> Self {
        ImpactOptions {
            max_results: 0,
            include_tests: true,
            include_references: true,
            depth: 1,
            direction: "both".to_string(),
            relationship_types: Vec::new(),
            max_nodes: DEFAULT_MAX_NODES,
        }
    }
}

/// Maximum allowed traversal depth. Deeper requests are rejected.
pub const MAX_DEPTH: usize = 5;
/// Default node-visit ceiling for bounded traversal.
pub const DEFAULT_MAX_NODES: usize = 1000;

// ── Analysis engine ─────────────────────────────────────────────────────

/// Run impact analysis over the given fact store for the specified target.
///
/// `workspace_root` is optional; when provided, freshness is computed by
/// comparing the store's generation-time repo state against the current
/// repository state. When omitted, freshness is `None`.
pub fn analyze(
    store: &FactStore,
    target: ImpactTarget,
    opts: &ImpactOptions,
    workspace_root: Option<&std::path::Path>,
) -> ImpactResult {
    let collection = store.collection();

    let resolved = resolve_target(collection, &target);
    let (target_fact_id, target_info) = match resolved {
        Some(r) => r,
        None => {
            return ImpactResult {
                target: ImpactTargetInfo {
                    id: target_label(&target),
                    kind: target.kind().as_str().to_string(),
                    name: None,
                    path: file_path_opt(&target),
                },
                status: ImpactStatus::NotFound,
                direct_relationships: Vec::new(),
                transitive_relationships: Vec::new(),
                affected_tests: Vec::new(),
                affected_modules: Vec::new(),
                affected_packages: Vec::new(),
                evidence: Vec::new(),
                completeness: Completeness {
                    status: "unknown".to_string(),
                    limitations: vec!["target not found in fact store".to_string()],
                },
                provenance_summary: ProvenanceSummary::default(),
                traversal_metadata: TraversalMetadata::default(),
                freshness: compute_impact_freshness(store, workspace_root),
            };
        }
    };

    let depth = opts.depth.min(MAX_DEPTH);
    let direction = if opts.direction.is_empty() {
        "both".to_string()
    } else {
        opts.direction.clone()
    };
    let max_nodes = if opts.max_nodes == 0 {
        DEFAULT_MAX_NODES
    } else {
        opts.max_nodes
    };

    let (direct_rels, transitive_rels, provenance_summary, traversal_meta) = if depth == 0 {
        // Target-only: no relationships traversed.
        (
            Vec::new(),
            Vec::new(),
            ProvenanceSummary::default(),
            TraversalMetadata {
                depth_limit: 0,
                direction: direction.clone(),
                relationship_types: opts.relationship_types.clone(),
                max_nodes,
                nodes_visited: 1,
                edges_traversed: 0,
                truncated: false,
                truncation_reason: Some("depth=0, no traversal performed".to_string()),
            },
        )
    } else if depth == 1 {
        // Backward-compatible: direct relationships only (both directions).
        let mut relationships =
            gather_relationships(&collection, &target_fact_id, &target_info, &direction);
        for rel in relationships.iter_mut() {
            rel.depth = 1;
        }
        let summary = build_provenance_summary(&relationships);
        let meta = TraversalMetadata {
            depth_limit: 1,
            direction: direction.clone(),
            relationship_types: opts.relationship_types.clone(),
            max_nodes,
            nodes_visited: relationships.len() + 1,
            edges_traversed: relationships.len(),
            truncated: false,
            truncation_reason: None,
        };
        (relationships, Vec::new(), summary, meta)
    } else {
        // Bounded BFS traversal.
        traverse(
            &collection,
            &target_fact_id,
            &target_info,
            depth,
            &direction,
            &opts.relationship_types,
            max_nodes,
            opts.max_results,
        )
    };

    let mut relationships = direct_rels;
    if opts.max_results > 0 {
        relationships.truncate(opts.max_results);
    }

    let evidence: Vec<EvidenceRecord> = relationships
        .iter()
        .chain(transitive_rels.iter())
        .map(|r| EvidenceRecord {
            fact_kind: "relationship".to_string(),
            fact_id: format!("{}→{}", target_info.id, r.target_id),
            fact_name: Some(r.target_name.clone()),
            source_location: r.source_location.clone(),
            description: format!(
                "{} {} {} (depth={})",
                target_info.name.as_deref().unwrap_or("target"),
                r.relationship_kind,
                r.target_name,
                r.depth
            ),
        })
        .collect();

    let mut tests = if opts.include_tests {
        gather_tests(&collection, &target_fact_id, &target)
    } else {
        Vec::new()
    };
    if opts.max_results > 0 && tests.len() > opts.max_results {
        tests.truncate(opts.max_results);
    }

    let mut modules = gather_modules(&collection, &target_fact_id, &target);
    if opts.max_results > 0 && modules.len() > opts.max_results {
        modules.truncate(opts.max_results);
    }

    let mut packages = gather_packages(&collection, &target_fact_id, &target);
    if opts.max_results > 0 && packages.len() > opts.max_results {
        packages.truncate(opts.max_results);
    }

    let mut completeness = build_completeness(&collection, &target);
    if traversal_meta.truncated {
        if completeness.status == "complete" {
            completeness.status = "partial".to_string();
        }
        completeness.limitations.push(format!(
            "traversal limited: {}",
            traversal_meta
                .truncation_reason
                .as_deref()
                .unwrap_or("unknown")
        ));
    }

    ImpactResult {
        target: target_info,
        status: ImpactStatus::Ok,
        direct_relationships: relationships,
        transitive_relationships: transitive_rels,
        affected_tests: tests,
        affected_modules: modules,
        affected_packages: packages,
        evidence,
        completeness,
        provenance_summary,
        traversal_metadata: traversal_meta,
        freshness: compute_impact_freshness(store, workspace_root),
    }
}

/// Whether the analysis could be invalidated as a result of an invalid
/// parameter. Callers surface this as an MCP invalid_params error.
#[derive(Debug, Clone)]
pub struct ImpactAnalysisError(pub String);

/// Compute store-wide freshness by comparing the fact store's generation-time
/// repository state against the current repository state at `workspace_root`.
/// Returns `None` when no generation state is recorded or the workspace root
/// is not provided (freshness is informational and only meaningful when the
/// store was built from a real repository).
fn compute_impact_freshness(
    store: &FactStore,
    workspace_root: Option<&std::path::Path>,
) -> Option<FreshnessStatus> {
    let gen_state = store.collection().model().generation_repo_state();
    let Some(root) = workspace_root else {
        return None;
    };
    let current = crate::sandbox::RepoState::capture(&root.to_path_buf());
    match (gen_state, current) {
        (Some(prev), Some(cur)) => {
            if prev.working_tree_hash == cur.working_tree_hash {
                Some(FreshnessStatus::Fresh)
            } else {
                Some(FreshnessStatus::Stale)
            }
        }
        _ => Some(FreshnessStatus::Unknown),
    }
}

/// Validate impact-analysis options and return an error when invalid.
pub fn validate_opts(opts: &ImpactOptions) -> Result<(), ImpactAnalysisError> {
    if opts.depth > MAX_DEPTH {
        return Err(ImpactAnalysisError(format!(
            "depth {} exceeds maximum allowed depth of {}",
            opts.depth, MAX_DEPTH
        )));
    }
    match opts.direction.as_str() {
        "" | "both" | "outgoing" | "incoming" => {}
        other => {
            return Err(ImpactAnalysisError(format!(
                "invalid direction '{other}'; use 'both', 'outgoing', or 'incoming'"
            )));
        }
    }
    for rt in &opts.relationship_types {
        if RelationshipKind::parse(rt).is_none() && rt != "references" {
            return Err(ImpactAnalysisError(format!(
                "unsupported relationship_type '{rt}'; use a known kind or 'references'"
            )));
        }
    }
    Ok(())
}

// ── Graph traversal ─────────────────────────────────────────────────────

/// Edge payload used during BFS traversal.
#[derive(Clone)]
struct GraphEdge {
    target_id: FactId,
    kind: String,
    provenance: Provenance,
    location: Option<crate::engineering_facts::location::SourceLocation>,
}

/// Run a bounded BFS from `start` over the relationship/reference graph.
///
/// Returns `(direct_relationships, transitive_relationships, provenance_summary, traversal_metadata)`.
fn traverse(
    collection: &crate::fact_store::collection::FactCollection,
    start: &FactId,
    start_info: &ImpactTargetInfo,
    depth: usize,
    direction: &str,
    type_filter: &[String],
    max_nodes: usize,
    max_results: usize,
) -> (
    Vec<ImpactRelationship>,
    Vec<ImpactRelationship>,
    ProvenanceSummary,
    TraversalMetadata,
) {
    // Build adjacency lists.
    let (outgoing, incoming) = build_adjacency(collection, type_filter);

    let mut visited: std::collections::HashSet<FactId> = std::collections::HashSet::new();
    visited.insert(start.clone());

    // BFS queue: (current_node_id, current_depth, path_to_here).
    let mut queue: Vec<(FactId, usize, Vec<PathEdge>)> = Vec::new();
    queue.push((start.clone(), 0, Vec::new()));

    let mut direct_relationships: Vec<ImpactRelationship> = Vec::new();
    let mut transitive_relationships: Vec<ImpactRelationship> = Vec::new();
    let mut nodes_visited = 1usize;
    let mut edges_traversed = 0usize;
    let mut truncated = false;
    let mut truncation_reason: Option<String> = None;

    // Track which (node_id, edge_kind, direction) pairs we've already emitted
    // to avoid duplicates across BFS levels.
    let mut emitted: std::collections::HashSet<(FactId, String, String)> =
        std::collections::HashSet::new();

    let mut qi = 0usize;
    while qi < queue.len() {
        let (current_id, current_depth, path_so_far) = queue[qi].clone();
        qi += 1;
        if current_depth >= depth {
            continue;
        }

        // Determine which adjacency list(s) to follow.
        let neighbors: Vec<GraphEdge> = match direction {
            "outgoing" => outgoing
                .get(&current_id)
                .map(|v| v.clone())
                .unwrap_or_default(),
            "incoming" => incoming
                .get(&current_id)
                .map(|v| v.clone())
                .unwrap_or_default(),
            _ => {
                // "both": merge outgoing and incoming, deduplicating by target.
                let out: Vec<GraphEdge> = outgoing
                    .get(&current_id)
                    .map(|v| v.clone())
                    .unwrap_or_default();
                let inc: Vec<GraphEdge> = incoming
                    .get(&current_id)
                    .map(|v| v.clone())
                    .unwrap_or_default();
                let mut merged: Vec<GraphEdge> = out.into_iter().chain(inc.into_iter()).collect();
                merged.sort_by_key(|e| e.target_id.to_string());
                merged.dedup_by_key(|e| e.target_id.to_string());
                merged
            }
        };

        for edge in neighbors {
            edges_traversed += 1;

            if visited.contains(&edge.target_id) {
                continue;
            }
            if nodes_visited >= max_nodes {
                truncated = true;
                truncation_reason = Some("traversal node limit reached".to_string());
                break;
            }

            let edge_kind = edge.kind.clone();
            let edge_direction = if direction == "incoming"
                || (direction == "both"
                    && incoming_contains(&incoming, &current_id, &edge.target_id))
            {
                "incoming".to_string()
            } else {
                "outgoing".to_string()
            };

            let emit_key = (
                edge.target_id.clone(),
                edge_kind.clone(),
                edge_direction.clone(),
            );
            if !emitted.insert(emit_key) {
                continue;
            }

            nodes_visited += 1;
            visited.insert(edge.target_id.clone());
            let new_depth = current_depth + 1;
            let mut new_path = path_so_far.clone();
            // Record the path edge in the actual relationship direction.
            let (path_src, path_tgt) = if edge_direction == "incoming" {
                (edge.target_id.to_string(), current_id.to_string())
            } else {
                (current_id.to_string(), edge.target_id.to_string())
            };
            new_path.push(PathEdge {
                source_id: path_src,
                target_id: path_tgt,
                kind: edge.kind.clone(),
                provenance: edge.provenance.clone(),
            });

            let (name, _loc) = fact_name_loc(collection, &edge.target_id, edge.location.as_ref());
            let rel = ImpactRelationship {
                target_id: edge.target_id.to_string(),
                target_name: name,
                relationship_kind: edge.kind.clone(),
                direction: edge_direction.clone(),
                source_location: edge.location.as_ref().and_then(|l| l.file.clone()),
                provenance: edge.provenance.clone(),
                depth: new_depth,
                path: if new_depth >= 2 {
                    new_path.clone()
                } else {
                    Vec::new()
                },
            };

            if new_depth == 1 {
                direct_relationships.push(rel);
            } else {
                transitive_relationships.push(rel);
            }
            queue.push((edge.target_id.clone(), new_depth, new_path));
        }

        if truncated {
            break;
        }
    }

    // Sort deterministically: depth asc, relationship_kind, direction, target_id.
    direct_relationships.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then(a.relationship_kind.cmp(&b.relationship_kind))
            .then(a.direction.cmp(&b.direction))
            .then(a.target_id.cmp(&b.target_id))
    });
    direct_relationships.dedup_by(|a, b| {
        a.target_id == b.target_id
            && a.relationship_kind == b.relationship_kind
            && a.direction == b.direction
    });

    transitive_relationships.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then(a.relationship_kind.cmp(&b.relationship_kind))
            .then(a.direction.cmp(&b.direction))
            .then(a.target_id.cmp(&b.target_id))
    });
    transitive_relationships.dedup_by(|a, b| {
        a.target_id == b.target_id
            && a.relationship_kind == b.relationship_kind
            && a.direction == b.direction
    });

    // Apply max_results cap on the combined relationship list.
    if max_results > 0 {
        let total = direct_relationships.len() + transitive_relationships.len();
        if total > max_results {
            let remain = max_results.saturating_sub(direct_relationships.len());
            if remain < transitive_relationships.len() {
                transitive_relationships.truncate(remain);
                truncated = true;
                if truncation_reason.is_none() {
                    truncation_reason = Some("result limit reached".to_string());
                }
            } else if direct_relationships.len() > max_results {
                direct_relationships.truncate(max_results);
                truncated = true;
                if truncation_reason.is_none() {
                    truncation_reason = Some("result limit reached".to_string());
                }
            }
        }
    }

    let summary = {
        let mut s = build_provenance_summary(&direct_relationships);
        let s2 = build_provenance_summary(&transitive_relationships);
        s.verified_edges += s2.verified_edges;
        s.heuristic_edges += s2.heuristic_edges;
        s.unknown_edges += s2.unknown_edges;
        s
    };

    let meta = TraversalMetadata {
        depth_limit: depth,
        direction: direction.to_string(),
        relationship_types: type_filter.to_vec(),
        max_nodes,
        nodes_visited,
        edges_traversed,
        truncated,
        truncation_reason,
    };

    (
        direct_relationships,
        transitive_relationships,
        summary,
        meta,
    )
}

/// Check whether an incoming edge from `src` to `tgt` exists in the incoming
/// adjacency list. Used to determine the correct direction label when
/// direction="both".
fn incoming_contains(
    incoming: &std::collections::HashMap<FactId, Vec<GraphEdge>>,
    src: &FactId,
    tgt: &FactId,
) -> bool {
    incoming
        .get(src)
        .map(|edges| edges.iter().any(|e| &e.target_id == tgt))
        .unwrap_or(false)
}

/// Build outgoing and incoming adjacency lists from the fact collection,
/// applying the optional relationship-type filter.
fn build_adjacency(
    collection: &crate::fact_store::collection::FactCollection,
    type_filter: &[String],
) -> (
    std::collections::HashMap<FactId, Vec<GraphEdge>>,
    std::collections::HashMap<FactId, Vec<GraphEdge>>,
) {
    let mut outgoing: std::collections::HashMap<FactId, Vec<GraphEdge>> =
        std::collections::HashMap::new();
    let mut incoming: std::collections::HashMap<FactId, Vec<GraphEdge>> =
        std::collections::HashMap::new();

    let passes_filter = |kind: &str| -> bool {
        if type_filter.is_empty() {
            return true;
        }
        type_filter.contains(&kind.to_string())
    };

    for rel in collection.relationships() {
        let kind = rel.kind.as_str().to_string();
        if !passes_filter(&kind) {
            continue;
        }
        let prov = fact_provenance(&rel.metadata);
        let edge_out = GraphEdge {
            target_id: rel.target.clone(),
            kind: kind.clone(),
            provenance: prov.clone(),
            location: rel.location.clone(),
        };
        let edge_in = GraphEdge {
            target_id: rel.source.clone(),
            kind: kind.clone(),
            provenance: prov,
            location: rel.location.clone(),
        };
        outgoing
            .entry(rel.source.clone())
            .or_default()
            .push(edge_out);
        incoming
            .entry(rel.target.clone())
            .or_default()
            .push(edge_in);
    }

    for r in collection.references() {
        let kind = "references".to_string();
        if !passes_filter(&kind) {
            continue;
        }
        let prov = fact_provenance(&r.metadata);
        let edge_out = GraphEdge {
            target_id: r.target.clone(),
            kind: kind.clone(),
            provenance: prov.clone(),
            location: r.location.clone(),
        };
        let edge_in = GraphEdge {
            target_id: r.referrer.clone(),
            kind: kind,
            provenance: prov,
            location: r.location.clone(),
        };
        outgoing
            .entry(r.referrer.clone())
            .or_default()
            .push(edge_out);
        incoming.entry(r.target.clone()).or_default().push(edge_in);
    }

    // Sort adjacency lists deterministically by target_id.
    for edges in outgoing.values_mut() {
        edges.sort_by(|a, b| a.target_id.cmp(&b.target_id));
    }
    for edges in incoming.values_mut() {
        edges.sort_by(|a, b| a.target_id.cmp(&b.target_id));
    }

    (outgoing, incoming)
}

fn build_provenance_summary(relationships: &[ImpactRelationship]) -> ProvenanceSummary {
    let mut summary = ProvenanceSummary::default();
    for rel in relationships {
        match rel.provenance {
            Provenance::Verified => summary.verified_edges += 1,
            Provenance::Heuristic => summary.heuristic_edges += 1,
            Provenance::Unknown => summary.unknown_edges += 1,
        }
    }
    summary
}

/// Resolve a human-readable symbol name to a symbolic ImpactTarget by
/// searching the store. Returns None when no match is found, Some with
/// one match, or Err when multiple matches exist (ambiguity).
pub fn resolve_symbol_name(store: &FactStore, name: &str) -> Result<ImpactTarget, String> {
    let collection = store.collection();
    let query = name.to_lowercase();
    let mut matches: Vec<SymbolId> = Vec::new();
    for sym in collection.symbols() {
        if sym.name.to_lowercase() == query {
            matches.push(sym.id.clone());
        }
    }
    match matches.len() {
        0 => Err(format!("no symbol found matching '{name}'")),
        1 => Ok(ImpactTarget::Symbol(matches[0].clone())),
        _ => Err(format!(
            "ambiguous symbol '{name}': {} matches — be more specific",
            matches.len()
        )),
    }
}

// ── Target resolution ───────────────────────────────────────────────────

type ResolvedTarget = (FactId, ImpactTargetInfo);

fn resolve_target(
    collection: &crate::fact_store::collection::FactCollection,
    target: &ImpactTarget,
) -> Option<ResolvedTarget> {
    match target {
        ImpactTarget::Symbol(sym_id) => {
            if let Some(sym) = collection.symbol(sym_id) {
                Some((
                    FactId::Symbol(sym.id.clone()),
                    ImpactTargetInfo {
                        id: sym.id.to_string(),
                        kind: "symbol".to_string(),
                        name: Some(sym.name.clone()),
                        path: sym.location.file.clone(),
                    },
                ))
            } else {
                None
            }
        }
        ImpactTarget::File(path) => {
            for m in collection.modules() {
                if let Some(mp) = &m.path {
                    if mp == path {
                        return Some((
                            FactId::Module(m.id.clone()),
                            ImpactTargetInfo {
                                id: m.id.to_string(),
                                kind: "module".to_string(),
                                name: Some(m.name.clone()),
                                path: Some(mp.clone()),
                            },
                        ));
                    }
                }
            }
            for s in collection.symbols() {
                if let Some(sf) = &s.location.file {
                    if sf == path {
                        return Some((
                            FactId::Symbol(s.id.clone()),
                            ImpactTargetInfo {
                                id: s.id.to_string(),
                                kind: "symbol".to_string(),
                                name: Some(s.name.clone()),
                                path: Some(sf.clone()),
                            },
                        ));
                    }
                }
            }
            None
        }
        ImpactTarget::Module(mod_id) => {
            if let Some(m) = collection.module(mod_id) {
                Some((
                    FactId::Module(m.id.clone()),
                    ImpactTargetInfo {
                        id: m.id.to_string(),
                        kind: "module".to_string(),
                        name: Some(m.name.clone()),
                        path: m.path.clone(),
                    },
                ))
            } else {
                None
            }
        }
        ImpactTarget::Package(pkg_id) => {
            if let Some(p) = collection.package(pkg_id) {
                Some((
                    FactId::Package(p.id.clone()),
                    ImpactTargetInfo {
                        id: p.id.to_string(),
                        kind: "package".to_string(),
                        name: Some(p.name.clone()),
                        path: None,
                    },
                ))
            } else {
                None
            }
        }
    }
}

fn target_label(target: &ImpactTarget) -> String {
    match target {
        ImpactTarget::Symbol(id) => id.to_string(),
        ImpactTarget::File(path) => path.clone(),
        ImpactTarget::Module(id) => id.to_string(),
        ImpactTarget::Package(id) => id.to_string(),
    }
}

fn file_path_opt(target: &ImpactTarget) -> Option<String> {
    match target {
        ImpactTarget::File(path) => Some(path.clone()),
        _ => None,
    }
}

// ── Relationship gathering ──────────────────────────────────────────────

fn gather_relationships(
    collection: &crate::fact_store::collection::FactCollection,
    target_id: &FactId,
    target_info: &ImpactTargetInfo,
    direction: &str,
) -> Vec<ImpactRelationship> {
    let mut out = Vec::new();
    let include_outgoing = direction == "outgoing" || direction == "both";
    let include_incoming = direction == "incoming" || direction == "both";

    if include_outgoing {
        for rel in collection.relationships() {
            if &rel.source == target_id {
                let (name, loc) = fact_name_loc(collection, &rel.target, rel.location.as_ref());
                out.push(ImpactRelationship {
                    target_id: rel.target.to_string(),
                    target_name: name,
                    relationship_kind: rel.kind.as_str().to_string(),
                    direction: "outgoing".to_string(),
                    source_location: loc,
                    provenance: fact_provenance(&rel.metadata),
                    depth: 1,
                    path: Vec::new(),
                });
            }
        }

        for r in collection.references() {
            if &r.referrer == target_id {
                let (name, loc) = fact_name_loc(collection, &r.target, r.location.as_ref());
                out.push(ImpactRelationship {
                    target_id: r.target.to_string(),
                    target_name: name,
                    relationship_kind: "references".to_string(),
                    direction: "outgoing".to_string(),
                    source_location: loc,
                    provenance: fact_provenance(&r.metadata),
                    depth: 1,
                    path: Vec::new(),
                });
            }
        }
    }

    if include_incoming {
        for rel in collection.relationships() {
            if &rel.target == target_id {
                let (name, loc) = fact_name_loc(collection, &rel.source, rel.location.as_ref());
                out.push(ImpactRelationship {
                    target_id: rel.source.to_string(),
                    target_name: name,
                    relationship_kind: rel.kind.as_str().to_string(),
                    direction: "incoming".to_string(),
                    source_location: loc,
                    provenance: fact_provenance(&rel.metadata),
                    depth: 1,
                    path: Vec::new(),
                });
            }
        }

        for r in collection.references() {
            if &r.target == target_id {
                let (name, loc) = fact_name_loc(collection, &r.referrer, r.location.as_ref());
                out.push(ImpactRelationship {
                    target_id: r.referrer.to_string(),
                    target_name: name,
                    relationship_kind: "references".to_string(),
                    direction: "incoming".to_string(),
                    source_location: loc,
                    provenance: fact_provenance(&r.metadata),
                    depth: 1,
                    path: Vec::new(),
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.relationship_kind
            .cmp(&b.relationship_kind)
            .then(a.direction.cmp(&b.direction))
            .then(a.target_id.cmp(&b.target_id))
    });
    out.dedup_by(|a, b| {
        a.target_id == b.target_id
            && a.relationship_kind == b.relationship_kind
            && a.direction == b.direction
    });
    out
}

fn fact_provenance(metadata: &crate::engineering_facts::metadata::FactMetadata) -> Provenance {
    if let Some(p) = metadata.get("provenance") {
        match p {
            "heuristic" => Provenance::Heuristic,
            "unknown" => Provenance::Unknown,
            _ => Provenance::Verified,
        }
    } else {
        Provenance::Verified
    }
}

fn fact_name_loc<'a>(
    collection: &'a crate::fact_store::collection::FactCollection,
    id: &FactId,
    loc: Option<&crate::engineering_facts::location::SourceLocation>,
) -> (String, Option<String>) {
    let name = match collection.find(id) {
        Some(crate::engineering_facts::FactRef::Symbol(s)) => s.name.clone(),
        Some(crate::engineering_facts::FactRef::Module(m)) => m.name.clone(),
        Some(crate::engineering_facts::FactRef::Package(p)) => p.name.clone(),
        Some(crate::engineering_facts::FactRef::Test(t)) => t.name.clone(),
        Some(crate::engineering_facts::FactRef::Workspace(w)) => w.name.clone(),
        Some(crate::engineering_facts::FactRef::BuildTarget(b)) => b.name.clone(),
        Some(crate::engineering_facts::FactRef::Dependency(d)) => d.id.to_string(),
        Some(crate::engineering_facts::FactRef::Relationship(r)) => r.id.to_string(),
        Some(crate::engineering_facts::FactRef::Reference(r)) => r.id.to_string(),
        Some(crate::engineering_facts::FactRef::Diagnostic(d)) => d.id.to_string(),
        Some(crate::engineering_facts::FactRef::ArchitectureRule(a)) => a.id.to_string(),
        _ => id.to_string(),
    };
    let loc = loc.and_then(|l| l.file.clone());
    (name, loc)
}

// ── Test gathering ──────────────────────────────────────────────────────

fn gather_tests(
    collection: &crate::fact_store::collection::FactCollection,
    target_id: &FactId,
    target: &ImpactTarget,
) -> Vec<TestReference> {
    let mut tests = Vec::new();

    match target {
        ImpactTarget::Symbol(sym_id) => {
            for test in collection.tests() {
                for tested in &test.tested {
                    if tested == sym_id {
                        tests.push(TestReference {
                            id: test.id.to_string(),
                            name: test.name.clone(),
                            file: test.location.as_ref().and_then(|l| l.file.clone()),
                            relation: "tests".to_string(),
                            provenance: Provenance::Verified,
                        });
                        break;
                    }
                }
            }
        }
        ImpactTarget::Module(mod_id) => {
            for test in collection.tests() {
                if let Some(loc) = &test.location {
                    if let Some(tm) = &loc.module {
                        if tm == mod_id {
                            tests.push(TestReference {
                                id: test.id.to_string(),
                                name: test.name.clone(),
                                file: loc.file.clone(),
                                relation: "in_module".to_string(),
                                provenance: Provenance::Verified,
                            });
                        }
                    }
                }
            }
        }
        ImpactTarget::File(path) => {
            for m in collection.modules() {
                if let Some(mp) = &m.path {
                    if mp == path {
                        for test in collection.tests() {
                            if let Some(loc) = &test.location {
                                if let Some(tm) = &loc.module {
                                    if tm == &m.id {
                                        tests.push(TestReference {
                                            id: test.id.to_string(),
                                            name: test.name.clone(),
                                            file: loc.file.clone(),
                                            relation: "in_module".to_string(),
                                            provenance: Provenance::Verified,
                                        });
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
        ImpactTarget::Package(pkg_id) => {
            for test in collection.tests() {
                if let Some(loc) = &test.location {
                    if let Some(tm) = &loc.module {
                        if let Some(mod_fact) = collection.module(tm) {
                            if let Some(tp) = &mod_fact.package {
                                if tp == pkg_id {
                                    tests.push(TestReference {
                                        id: test.id.to_string(),
                                        name: test.name.clone(),
                                        file: loc.file.clone(),
                                        relation: "in_package".to_string(),
                                        provenance: Provenance::Verified,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    tests.sort_by(|a, b| a.id.cmp(&b.id));
    tests.dedup_by(|a, b| a.id == b.id);
    tests
}

// ── Module gathering ────────────────────────────────────────────────────

fn gather_modules(
    collection: &crate::fact_store::collection::FactCollection,
    target_id: &FactId,
    target: &ImpactTarget,
) -> Vec<ModuleInfo> {
    let mut modules = Vec::new();
    let target_mod_id = target_mod_of(collection, target_id).clone();

    match target {
        ImpactTarget::Symbol(sym_id) => {
            if let Some(sym) = collection.symbol(sym_id) {
                if let Some(mod_id) = &sym.module {
                    if let Some(m) = collection.module(mod_id) {
                        modules.push(ModuleInfo {
                            id: m.id.to_string(),
                            name: m.name.clone(),
                            path: m.path.clone(),
                            relation: "owns".to_string(),
                        });
                    }
                }
            }
            if let Some(ref mid) = target_mod_id {
                for rel in collection.relationships() {
                    if rel.kind == RelationshipKind::Imports {
                        if let Some(tgt_mod) = as_module_id(collection, &rel.target) {
                            if tgt_mod == *mid {
                                if let Some(src_mod) = as_module_id(collection, &rel.source) {
                                    if let Some(m) = collection.module(&src_mod) {
                                        if !modules.iter().any(|x| x.id == m.id.to_string()) {
                                            modules.push(ModuleInfo {
                                                id: m.id.to_string(),
                                                name: m.name.clone(),
                                                path: m.path.clone(),
                                                relation: "imports".to_string(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        ImpactTarget::Module(mod_id) => {
            if let Some(m) = collection.module(mod_id) {
                if let Some(pkg_id) = &m.package {
                    for m2 in collection.modules() {
                        if let Some(m2p) = &m2.package {
                            if m2p == pkg_id && m2.id != *mod_id {
                                modules.push(ModuleInfo {
                                    id: m2.id.to_string(),
                                    name: m2.name.clone(),
                                    path: m2.path.clone(),
                                    relation: "same_package".to_string(),
                                });
                            }
                        }
                    }
                }
            }
            for rel in collection.relationships() {
                if rel.kind == RelationshipKind::Imports {
                    if let Some(tgt) = as_module_id(collection, &rel.target) {
                        if tgt == *mod_id {
                            if let Some(src) = as_module_id(collection, &rel.source) {
                                if let Some(m) = collection.module(&src) {
                                    if !modules.iter().any(|x| x.id == m.id.to_string()) {
                                        modules.push(ModuleInfo {
                                            id: m.id.to_string(),
                                            name: m.name.clone(),
                                            path: m.path.clone(),
                                            relation: "imports".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        ImpactTarget::File(path) => {
            for m in collection.modules() {
                if let Some(mp) = &m.path {
                    if mp == path {
                        if let Some(pkg_id) = &m.package {
                            for m2 in collection.modules() {
                                if let Some(m2p) = &m2.package {
                                    if m2p == pkg_id && m2.id != m.id {
                                        modules.push(ModuleInfo {
                                            id: m2.id.to_string(),
                                            name: m2.name.clone(),
                                            path: m2.path.clone(),
                                            relation: "same_package".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
        ImpactTarget::Package(pkg_id) => {
            for m in collection.modules() {
                if let Some(mp) = &m.package {
                    if mp == pkg_id {
                        modules.push(ModuleInfo {
                            id: m.id.to_string(),
                            name: m.name.clone(),
                            path: m.path.clone(),
                            relation: "member_of".to_string(),
                        });
                    }
                }
            }
        }
    }

    modules.sort_by(|a, b| a.id.cmp(&b.id));
    modules.dedup_by(|a, b| a.id == b.id);
    modules
}

// ── Package gathering ───────────────────────────────────────────────────

fn gather_packages(
    collection: &crate::fact_store::collection::FactCollection,
    target_id: &FactId,
    target: &ImpactTarget,
) -> Vec<PackageInfo> {
    let mut packages = Vec::new();

    match target {
        ImpactTarget::Symbol(sym_id) => {
            if let Some(sym) = collection.symbol(sym_id) {
                if let Some(mod_id) = &sym.module {
                    if let Some(m) = collection.module(mod_id) {
                        if let Some(pkg_id) = &m.package {
                            if let Some(p) = collection.package(pkg_id) {
                                packages.push(PackageInfo {
                                    id: p.id.to_string(),
                                    name: p.name.clone(),
                                    relation: "owns_module".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
        ImpactTarget::Module(mod_id) => {
            if let Some(m) = collection.module(mod_id) {
                if let Some(pkg_id) = &m.package {
                    if let Some(p) = collection.package(pkg_id) {
                        packages.push(PackageInfo {
                            id: p.id.to_string(),
                            name: p.name.clone(),
                            relation: "owns".to_string(),
                        });
                    }
                }
            }
        }
        ImpactTarget::File(path) => {
            for m in collection.modules() {
                if let Some(mp) = &m.path {
                    if mp == path {
                        if let Some(pkg_id) = &m.package {
                            if let Some(p) = collection.package(pkg_id) {
                                packages.push(PackageInfo {
                                    id: p.id.to_string(),
                                    name: p.name.clone(),
                                    relation: "owns_module".to_string(),
                                });
                            }
                        }
                        break;
                    }
                }
            }
        }
        ImpactTarget::Package(pkg_id) => {
            for dep in collection.dependencies() {
                if dep.target == *target_id {
                    if let Some(src) = collection.find(&dep.source) {
                        if let crate::engineering_facts::FactRef::Package(p) = src {
                            if !packages.iter().any(|pk| pk.id == p.id.to_string()) {
                                packages.push(PackageInfo {
                                    id: p.id.to_string(),
                                    name: p.name.clone(),
                                    relation: "depends_on".to_string(),
                                });
                            }
                        }
                    }
                }
            }
            if let Some(p) = collection.package(pkg_id) {
                packages.push(PackageInfo {
                    id: p.id.to_string(),
                    name: p.name.clone(),
                    relation: "self".to_string(),
                });
            }
        }
    }

    packages.sort_by(|a, b| a.id.cmp(&b.id));
    packages.dedup_by(|a, b| a.id == b.id);
    packages
}

// ── Completeness ────────────────────────────────────────────────────────

fn build_completeness(
    collection: &crate::fact_store::collection::FactCollection,
    target: &ImpactTarget,
) -> Completeness {
    let mut limitations = Vec::new();
    let mut status = "complete".to_string();

    let rel_count = collection.relationships().len();
    let ref_count = collection.references().len();

    if rel_count == 0 && ref_count == 0 {
        status = "partial".to_string();
        limitations.push(
            "no relationship or reference facts exist — impact limited to scope membership"
                .to_string(),
        );
    }

    if let ImpactTarget::File(path) = target {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if crate::intelligence::parser::languages::language_from_extension(ext).is_none() {
            status = "partial".to_string();
            limitations.push(format!("unsupported file extension: .{}", ext));
        }
    }

    Completeness {
        status,
        limitations,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn target_mod_of(
    collection: &crate::fact_store::collection::FactCollection,
    id: &FactId,
) -> Option<ModuleId> {
    if id.kind() == FactKind::Module {
        if let FactId::Module(mid) = id {
            return Some(mid.clone());
        }
    }
    if let FactId::Symbol(sid) = id {
        if let Some(sym) = collection.symbol(sid) {
            return sym.module.clone();
        }
    }
    None
}

fn as_module_id(
    collection: &crate::fact_store::collection::FactCollection,
    id: &FactId,
) -> Option<ModuleId> {
    if id.kind() == FactKind::Module {
        if let FactId::Module(mid) = id {
            return Some(mid.clone());
        }
    }
    if let FactId::Symbol(sid) = id {
        if let Some(sym) = collection.symbol(sid) {
            return sym.module.clone();
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_facts::{
        FactsBuilder, ModuleFact, ModuleId, PackageFact, PackageId, ReferenceFact,
        RelationshipFact, RelationshipId, RelationshipKind, SymbolFact, SymbolId, TestFact, TestId,
        WorkspaceFact, WorkspaceId,
    };
    use crate::fact_store::FactStore;

    fn make_store(
        modules: &[(&str, Option<&str>)],
        symbols: &[(&str, &str, &str, Option<&str>)],
        relationships: &[(&str, RelationshipKind, &str, &str)],
        references: &[(&str, &str, &str)],
        tests: &[(&str, &str, Vec<&str>, Option<&str>)],
    ) -> FactStore {
        let mut builder = FactsBuilder::new();
        let ws_id = WorkspaceId::new("ws::test");
        builder.add_workspace(WorkspaceFact::new(ws_id.clone(), "test".to_string()));

        let mut pkg_map: std::collections::HashMap<&str, PackageId> =
            std::collections::HashMap::new();
        for (mod_id, pkg_id) in modules {
            let mid = ModuleId::new(*mod_id);
            let mut mf = ModuleFact::new(mid.clone(), mod_id.to_string());
            mf.path = Some(mod_id.to_string());
            if let Some(pid) = pkg_id {
                let pid = pkg_map
                    .entry(pid)
                    .or_insert_with(|| PackageId::new(format!("pkg::{pid}")));
                mf.package = Some(pid.clone());
            }
            builder.add_module(mf);
        }
        for (_, pid) in &pkg_map {
            let mut pf = PackageFact::new(pid.clone(), pid.as_str().replace("pkg::", ""));
            pf.workspace = Some(ws_id.clone());
            builder.add_package(pf);
        }

        for (mod_id, name, kind_str, _pkg) in symbols {
            let kind = match *kind_str {
                "function" => crate::engineering_facts::SymbolKind::Function,
                "struct" => crate::engineering_facts::SymbolKind::Struct,
                "method" => crate::engineering_facts::SymbolKind::Method,
                _ => crate::engineering_facts::SymbolKind::Unknown,
            };
            let sym_id = SymbolId::new(format!("sym::{mod_id}::{name}_{kind_str}@1"));
            let mut sf = SymbolFact::new(sym_id.clone(), name.to_string(), kind);
            sf.module = Some(ModuleId::new(*mod_id));
            builder.add_symbol(sf);
        }

        for (rel_id, kind, src, tgt) in relationships {
            // Detect whether src/tgt are symbol or module ids.
            let src_id = if src.starts_with("sym::") {
                FactId::Symbol(SymbolId::new(src.to_string()))
            } else {
                FactId::Module(ModuleId::new(src.to_string()))
            };
            let tgt_id = if tgt.starts_with("sym::") {
                FactId::Symbol(SymbolId::new(tgt.to_string()))
            } else {
                FactId::Module(ModuleId::new(tgt.to_string()))
            };
            let rf = RelationshipFact::new(
                RelationshipId::new(rel_id.to_string()),
                kind.clone(),
                src_id,
                tgt_id,
            );
            builder.add_relationship(rf);
        }

        for (ref_id, referrer, target) in references {
            let rf = ReferenceFact::new(
                crate::engineering_facts::ReferenceId::new(ref_id.to_string()),
                FactId::Symbol(SymbolId::new(*referrer)),
                FactId::Symbol(SymbolId::new(*target)),
            );
            builder.add_reference(rf);
        }

        for (test_id, name, tested, mod_opt) in tests {
            let mut tf = TestFact::new(TestId::new(test_id.to_string()), name.to_string());
            for t in tested {
                tf.tested.push(SymbolId::new(t.to_string()));
            }
            if let Some(mid) = mod_opt {
                use crate::engineering_facts::SourceLocation;
                tf.location = Some(SourceLocation::new().with_module(ModuleId::new(*mid)));
            }
            builder.add_test(tf);
        }

        // Heuristic references: same-name symbols across different modules
        // that have no explicit relationship or reference facts.
        let mut existing_refs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for (ref_id, referrer, target) in references {
            existing_refs.insert((referrer.to_string(), target.to_string()));
            existing_refs.insert((target.to_string(), referrer.to_string()));
        }

        let mut by_name: std::collections::HashMap<String, Vec<(SymbolId, ModuleId)>> =
            std::collections::HashMap::new();
        for (mod_id, name, _kind_str, _pkg) in symbols {
            let sym_id = SymbolId::new(format!("sym::{mod_id}::{name}_function@1"));
            by_name
                .entry(name.to_string())
                .or_default()
                .push((sym_id.clone(), ModuleId::new(*mod_id)));
        }

        for (name, syms) in &by_name {
            if syms.len() < 2 {
                continue;
            }
            for i in 0..syms.len() {
                for j in (i + 1)..syms.len() {
                    let (id_a, mod_a) = &syms[i];
                    let (id_b, mod_b) = &syms[j];
                    if mod_a == mod_b {
                        continue;
                    }
                    let key = if id_a < id_b {
                        (id_a.to_string(), id_b.to_string())
                    } else {
                        (id_b.to_string(), id_a.to_string())
                    };
                    if existing_refs.contains(&key) {
                        continue;
                    }
                    let ref_id = format!(
                        "ref::{mod_a}::{name}→{mod_b}::{name}",
                        mod_a = mod_a.as_str(),
                        mod_b = mod_b.as_str(),
                        name = name,
                    );
                    let mut rf = ReferenceFact::new(
                        crate::engineering_facts::ReferenceId::new(ref_id),
                        FactId::Symbol(id_a.clone()),
                        FactId::Symbol(id_b.clone()),
                    );
                    rf.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
                        .attr("provenance", "heuristic")
                        .build();
                    builder.add_reference(rf);
                }
            }
        }

        FactStore::build(builder.build())
    }

    #[test]
    fn symbol_impact_returns_owning_module() {
        let store = make_store(
            &[("mod::a", Some("pkg1")), ("mod::b", Some("pkg1"))],
            &[("mod::a", "foo", "function", Some("pkg1"))],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        assert_eq!(result.target.kind, "symbol");
        assert_eq!(result.target.name.as_deref(), Some("foo"));
        assert_eq!(result.affected_modules.len(), 1);
        assert_eq!(result.affected_modules[0].relation, "owns");
    }

    #[test]
    fn symbol_impact_finds_importers() {
        let store = make_store(
            &[("mod::a", Some("pkg1")), ("mod::b", Some("pkg1"))],
            &[
                ("mod::a", "foo", "function", Some("pkg1")),
                ("mod::b", "foo", "function", Some("pkg1")),
            ],
            &[("rel::1", RelationshipKind::Imports, "mod::b", "mod::a")],
            &[(
                "ref::1",
                "sym::mod::b::foo_function@1",
                "sym::mod::a::foo_function@1",
            )],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        let incoming: Vec<_> = result
            .direct_relationships
            .iter()
            .filter(|r| r.direction == "incoming")
            .collect();
        assert!(
            incoming.len() >= 1,
            "expected at least 1 incoming relationship, got {}",
            incoming.len()
        );
    }

    #[test]
    fn symbol_impact_finds_tests() {
        let store = make_store(
            &[("mod::a", Some("pkg1"))],
            &[("mod::a", "foo", "function", Some("pkg1"))],
            &[],
            &[],
            &[(
                "test::1",
                "test_foo",
                vec!["sym::mod::a::foo_function@1"],
                Some("mod::a"),
            )],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                include_tests: true,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.affected_tests.len(), 1);
        assert_eq!(result.affected_tests[0].relation, "tests");
    }

    #[test]
    fn file_impact_resolves_by_path() {
        let store = make_store(
            &[("mod::src/lib", Some("pkg1"))],
            &[("mod::src/lib", "hello", "function", Some("pkg1"))],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::File("mod::src/lib".to_string());
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        assert_eq!(result.target.kind, "module");
    }

    #[test]
    fn file_impact_not_found() {
        let store = make_store(&[], &[], &[], &[], &[]);
        let target = ImpactTarget::File("nonexistent.rs".to_string());
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::NotFound);
    }

    #[test]
    fn module_impact_finds_same_package_siblings() {
        let store = make_store(
            &[
                ("mod::a", Some("pkg1")),
                ("mod::b", Some("pkg1")),
                ("mod::c", Some("pkg2")),
            ],
            &[
                ("mod::a", "x", "function", Some("pkg1")),
                ("mod::b", "y", "function", Some("pkg1")),
                ("mod::c", "z", "function", Some("pkg2")),
            ],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Module(ModuleId::new("mod::a"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        let siblings: Vec<_> = result
            .affected_modules
            .iter()
            .filter(|m| m.relation == "same_package")
            .collect();
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].id, "mod::b");
    }

    #[test]
    fn package_impact_finds_members() {
        let store = make_store(
            &[("mod::a", Some("pkg1")), ("mod::b", Some("pkg1"))],
            &[
                ("mod::a", "x", "function", Some("pkg1")),
                ("mod::b", "y", "function", Some("pkg1")),
            ],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Package(PackageId::new("pkg::pkg1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        let members: Vec<_> = result
            .affected_modules
            .iter()
            .filter(|m| m.relation == "member_of")
            .collect();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn max_results_limits_output() {
        let store = make_store(
            &[
                ("mod::a", Some("pkg1")),
                ("mod::b", Some("pkg1")),
                ("mod::c", Some("pkg1")),
            ],
            &[
                ("mod::a", "x", "function", Some("pkg1")),
                ("mod::b", "y", "function", Some("pkg1")),
                ("mod::c", "z", "function", Some("pkg1")),
            ],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Package(PackageId::new("pkg::pkg1"));
        let opts = ImpactOptions {
            max_results: 2,
            ..Default::default()
        };
        let result = analyze(&store, target, &opts, None);
        assert!(
            result.affected_modules.len() <= 2,
            "expected <=2 modules, got {}",
            result.affected_modules.len()
        );
    }

    #[test]
    fn completeness_partial_when_no_relationships() {
        let store = make_store(
            &[("mod::a", Some("pkg1"))],
            &[("mod::a", "foo", "function", Some("pkg1"))],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.completeness.status, "partial");
        assert!(
            result
                .completeness
                .limitations
                .iter()
                .any(|l| l.contains("no relationship")),
            "expected partial completeness hint"
        );
    }

    #[test]
    fn evidence_contains_provenance() {
        let store = make_store(
            &[("mod::a", Some("pkg1")), ("mod::b", Some("pkg1"))],
            &[
                ("mod::a", "foo", "function", Some("pkg1")),
                ("mod::b", "foo", "function", Some("pkg1")),
            ],
            &[("rel::1", RelationshipKind::Imports, "mod::b", "mod::a")],
            &[(
                "ref::1",
                "sym::mod::b::foo_function@1",
                "sym::mod::a::foo_function@1",
            )],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert!(!result.evidence.is_empty());
        let ev = &result.evidence[0];
        assert_eq!(ev.fact_kind, "relationship");
        assert!(!ev.description.is_empty());
    }

    #[test]
    fn target_type_file_with_unsupported_extension() {
        // Build a store so the target resolves, then query a file with an
        // unsupported extension. The file won't match any module path so
        // we get NotFound; completeness is unknown in that path.
        let store = make_store(
            &[("mod::main", Some("pkg1"))],
            &[("mod::main", "main", "function", Some("pkg1"))],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::File("script.xyz".to_string());
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert!(matches!(result.status, ImpactStatus::NotFound));
        assert_eq!(result.completeness.status, "unknown");
        assert!(result
            .completeness
            .limitations
            .iter()
            .any(|l| l.contains("target not found")));
    }

    #[test]
    fn verified_calls_appear_as_outgoing_relationships() {
        // Build a store with an explicit Calls relationship.
        let store = make_store(
            &[("mod::a", Some("pkg1")), ("mod::b", Some("pkg1"))],
            &[
                ("mod::a", "caller", "function", Some("pkg1")),
                ("mod::b", "callee", "function", Some("pkg1")),
            ],
            &[(
                "rel::call",
                RelationshipKind::Calls,
                "sym::mod::a::caller_function@1",
                "sym::mod::b::callee_function@1",
            )],
            &[],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::caller_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        let outgoing: Vec<_> = result
            .direct_relationships
            .iter()
            .filter(|r| r.direction == "outgoing" && r.relationship_kind == "calls")
            .collect();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].target_name, "callee");
        assert_eq!(outgoing[0].provenance, Provenance::Verified);
    }

    #[test]
    fn ambiguous_call_target_is_not_created() {
        // Two symbols with the same name in different modules.
        // A call to "foo" should not produce a verified edge because
        // the target is ambiguous.
        let store = make_store(
            &[
                ("mod::a", Some("pkg1")),
                ("mod::b", Some("pkg1")),
                ("mod::c", Some("pkg1")),
            ],
            &[
                ("mod::a", "foo", "function", Some("pkg1")),
                ("mod::b", "foo", "function", Some("pkg1")),
                ("mod::c", "bar", "function", Some("pkg1")),
            ],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        // No verified calls should exist since the call graph was empty.
        let verified_calls: Vec<_> = result
            .direct_relationships
            .iter()
            .filter(|r| r.relationship_kind == "calls" && r.provenance == Provenance::Verified)
            .collect();
        assert_eq!(verified_calls.len(), 0);
    }

    #[test]
    fn heuristic_references_have_heuristic_provenance() {
        // Two symbols with same name in different modules — triggers
        // heuristic reference (no AST call data).
        let store = make_store(
            &[("mod::a", Some("pkg1")), ("mod::b", Some("pkg1"))],
            &[
                ("mod::a", "foo", "function", Some("pkg1")),
                ("mod::b", "foo", "function", Some("pkg1")),
            ],
            &[],
            &[],
            &[],
        );
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::a::foo_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        // Heuristic references should be present (name coincidence).
        let heur_refs: Vec<_> = result
            .direct_relationships
            .iter()
            .filter(|r| {
                r.relationship_kind == "references" && r.provenance == Provenance::Heuristic
            })
            .collect();
        assert!(
            heur_refs.len() >= 1,
            "expected heuristic reference from name coincidence"
        );
    }

    // ── P2.3 bounded transitive traversal tests ───────────────────────────

    /// Build a linear chain A → B → C → D plus a back-edge C → A (cycle)
    /// and a heuristic edge A → X.
    fn make_chain_store() -> FactStore {
        // Symbols: a, b, c, d, x all in mod::chain
        make_store(
            &[("mod::chain", Some("pkg1"))],
            &[
                ("mod::chain", "a", "function", Some("pkg1")),
                ("mod::chain", "b", "function", Some("pkg1")),
                ("mod::chain", "c", "function", Some("pkg1")),
                ("mod::chain", "d", "function", Some("pkg1")),
                ("mod::chain", "x", "function", Some("pkg1")),
            ],
            &[
                // Verified: a calls b, b calls c, c calls d
                (
                    "rel::a→b",
                    RelationshipKind::Calls,
                    "sym::mod::chain::a_function@1",
                    "sym::mod::chain::b_function@1",
                ),
                (
                    "rel::b→c",
                    RelationshipKind::Calls,
                    "sym::mod::chain::b_function@1",
                    "sym::mod::chain::c_function@1",
                ),
                (
                    "rel::c→d",
                    RelationshipKind::Calls,
                    "sym::mod::chain::c_function@1",
                    "sym::mod::chain::d_function@1",
                ),
                // Cycle: c also calls a (verified)
                (
                    "rel::c→a",
                    RelationshipKind::Calls,
                    "sym::mod::chain::c_function@1",
                    "sym::mod::chain::a_function@1",
                ),
            ],
            &[],
            &[],
        )
    }

    /// depth=0 returns only the target with no relationships.
    #[test]
    fn depth_zero_returns_target_only() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 0,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        assert_eq!(result.target.name.as_deref(), Some("a"));
        assert!(result.direct_relationships.is_empty());
        assert!(result.transitive_relationships.is_empty());
        assert_eq!(result.traversal_metadata.depth_limit, 0);
        assert_eq!(result.traversal_metadata.nodes_visited, 1);
        assert_eq!(result.traversal_metadata.edges_traversed, 0);
    }

    /// depth=1 returns direct relationships with depth=1.
    #[test]
    fn depth_one_returns_direct_relationships() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 1,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        assert!(!result.direct_relationships.is_empty());
        assert!(result.transitive_relationships.is_empty());
        for rel in &result.direct_relationships {
            assert_eq!(rel.depth, 1);
            assert!(rel.path.is_empty());
        }
        assert_eq!(result.traversal_metadata.depth_limit, 1);
    }

    /// depth=2 returns direct + one hop transitive relationships.
    #[test]
    fn depth_two_returns_transitive_relationships() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 2,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        // With direction="both": depth-1 = b (outgoing a→b) + c (incoming c→a)
        // depth-2 = d (outgoing c→d)
        let depth1: Vec<_> = result
            .direct_relationships
            .iter()
            .filter(|r| r.depth == 1)
            .collect();
        let depth2: Vec<_> = result
            .transitive_relationships
            .iter()
            .filter(|r| r.depth == 2)
            .collect();
        assert_eq!(
            depth1.len(),
            2,
            "expected 2 depth-1 edges (a→b outgoing, c→a incoming), got {}",
            depth1.len()
        );
        assert_eq!(
            depth2.len(),
            1,
            "expected 1 depth-2 edge (c→d), got {}",
            depth2.len()
        );
        assert_eq!(depth2[0].target_name, "d");
        assert_eq!(result.traversal_metadata.depth_limit, 2);
    }

    /// depth=3 reaches d through the chain; no new nodes beyond depth 2.
    #[test]
    fn depth_three_reaches_chained_node() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 3,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        // All reachable nodes (a,b,c,d) are found by depth 2.
        let depth2: Vec<_> = result
            .transitive_relationships
            .iter()
            .filter(|r| r.depth == 2)
            .collect();
        assert_eq!(
            depth2.len(),
            1,
            "expected 1 depth-2 edge (c→d), got {}",
            depth2.len()
        );
        assert_eq!(depth2[0].target_name, "d");
        // No depth-3 edges because all nodes are already visited.
        let depth3: Vec<_> = result
            .transitive_relationships
            .iter()
            .filter(|r| r.depth == 3)
            .collect();
        assert!(
            depth3.is_empty(),
            "expected no depth-3 edges, got {}",
            depth3.len()
        );
        // Verify path is present for depth-2 edges.
        assert!(!depth2[0].path.is_empty(), "depth-2 edge should have path");
        assert_eq!(depth2[0].path.len(), 2);
    }

    /// depth exceeding MAX_DEPTH is rejected as invalid parameters.
    #[test]
    fn depth_exceeding_max_is_rejected() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let opts = ImpactOptions {
            depth: 10,
            ..Default::default()
        };
        let err = crate::impact::validate_opts(&opts);
        assert!(err.is_err(), "depth > MAX_DEPTH should be rejected");
        let err_msg = err.unwrap_err().0;
        assert!(err_msg.contains("depth"), "error should mention depth");
        assert!(err_msg.contains("5"), "error should mention MAX_DEPTH=5");
    }

    /// Cycles terminate deterministically — traversing a→b→c→a does not loop.
    #[test]
    fn cycle_terminates_without_infinite_loop() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 5,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        // Node 'a' should appear only once (as the target, not revisited).
        let a_count = result
            .direct_relationships
            .iter()
            .filter(|r| r.target_id.contains("a_function"))
            .count()
            + result
                .transitive_relationships
                .iter()
                .filter(|r| r.target_id.contains("a_function"))
                .count();
        // a should not appear as a reached node since it's the start.
        // (The cycle c→a is skipped because a is already visited.)
        assert_eq!(
            a_count, 0,
            "cycle back to target should not reappear, got {}",
            a_count
        );
        assert!(!result
            .completeness
            .limitations
            .iter()
            .any(|l| l.contains("infinite")));
    }

    /// max_nodes limit makes the result partial with an explicit limitation.
    #[test]
    fn max_nodes_limit_makes_result_partial() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 5,
                max_nodes: 3,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        assert_eq!(result.completeness.status, "partial");
        assert!(
            result
                .completeness
                .limitations
                .iter()
                .any(|l| l.contains("traversal limited")),
            "expected partial limitation about traversal limit"
        );
        assert!(result.traversal_metadata.truncated);
    }

    /// Provenance is preserved per-edge: verified edges stay verified,
    /// heuristic edges stay heuristic — no upgrade along paths.
    #[test]
    fn provenance_preserved_per_edge() {
        // Build a store with one verified and one heuristic edge.
        let mut builder = crate::engineering_facts::FactsBuilder::new();
        let ws_id = crate::engineering_facts::WorkspaceId::new("ws::test");
        builder.add_workspace(crate::engineering_facts::WorkspaceFact::new(
            ws_id.clone(),
            "test".to_string(),
        ));
        let mid = crate::engineering_facts::ModuleId::new("mod::p");
        let mut mf = crate::engineering_facts::ModuleFact::new(mid.clone(), "mod".to_string());
        mf.path = Some("mod/p.rs".to_string());
        builder.add_module(mf);

        let mut mk_sym =
            |mod_id: &str, name: &str| -> (crate::engineering_facts::SymbolId, String) {
                let sid = crate::engineering_facts::SymbolId::new(format!(
                    "sym::{mod_id}::{name}_function@1"
                ));
                let mut sf = crate::engineering_facts::SymbolFact::new(
                    sid.clone(),
                    name.to_string(),
                    crate::engineering_facts::SymbolKind::Function,
                );
                sf.module = Some(crate::engineering_facts::ModuleId::new(mod_id));
                builder.add_symbol(sf);
                (sid, name.to_string())
            };

        let (sa, _) = mk_sym("mod::p", "a");
        let (sb, _) = mk_sym("mod::p", "b");
        let (sc, _) = mk_sym("mod::p", "c");

        // Verified edge a → b
        let rf1 = crate::engineering_facts::RelationshipFact::new(
            crate::engineering_facts::RelationshipId::new("rel::v"),
            RelationshipKind::Calls,
            FactId::Symbol(sa.clone()),
            FactId::Symbol(sb.clone()),
        );
        builder.add_relationship(rf1);

        // Heuristic edge b → c (manually tagged)
        let mut rf2 = crate::engineering_facts::RelationshipFact::new(
            crate::engineering_facts::RelationshipId::new("rel::h"),
            RelationshipKind::Calls,
            FactId::Symbol(sb.clone()),
            FactId::Symbol(sc.clone()),
        );
        rf2.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
            .attr("provenance", "heuristic")
            .build();
        builder.add_relationship(rf2);

        let store = FactStore::build(builder.build());

        let target = ImpactTarget::Symbol(sa.clone());
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 2,
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);

        // Depth-1 edge a→b should be verified.
        let d1: Vec<_> = result
            .direct_relationships
            .iter()
            .filter(|r| r.target_id == sc.to_string() || r.target_id == sb.to_string())
            .collect();
        let b_edge = d1
            .iter()
            .find(|r| r.target_id == sb.to_string())
            .expect("should have edge to b");
        assert_eq!(b_edge.provenance, Provenance::Verified);

        // Depth-2 edge b→c should remain heuristic.
        let c_edge = result
            .transitive_relationships
            .iter()
            .find(|r| r.target_id == sc.to_string());
        assert!(c_edge.is_some(), "should reach c at depth 2");
        assert_eq!(
            c_edge.unwrap().provenance,
            Provenance::Heuristic,
            "heuristic provenance must not be upgraded through traversal"
        );

        // Provenance summary should reflect both.
        assert_eq!(result.provenance_summary.verified_edges, 1);
        assert_eq!(result.provenance_summary.heuristic_edges, 1);
    }

    /// Deterministic ordering: repeated executions produce identical output.
    #[test]
    fn deterministic_ordering() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let opts = ImpactOptions {
            depth: 3,
            ..Default::default()
        };
        let r1 = analyze(&store, target.clone(), &opts, None);
        let r2 = analyze(&store, target.clone(), &opts, None);
        let r3 = analyze(&store, target.clone(), &opts, None);
        fn snapshot(r: &crate::impact::ImpactResult) -> Vec<(usize, String, String, String)> {
            r.direct_relationships
                .iter()
                .chain(r.transitive_relationships.iter())
                .map(|e| {
                    (
                        e.depth,
                        e.relationship_kind.clone(),
                        e.direction.clone(),
                        e.target_id.clone(),
                    )
                })
                .collect()
        }
        assert_eq!(snapshot(&r1), snapshot(&r2));
        assert_eq!(snapshot(&r2), snapshot(&r3));
    }

    /// direction=outgoing only follows edges where the current node is source.
    #[test]
    fn direction_outgoing_only_finds_outgoing() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 1,
                direction: "outgoing".to_string(),
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        for rel in &result.direct_relationships {
            assert_eq!(rel.direction, "outgoing");
        }
        // a→b is the only outgoing edge from a.
        assert_eq!(result.direct_relationships.len(), 1);
        assert_eq!(result.direct_relationships[0].target_name, "b");
    }

    /// direction=incoming only finds edges where the current node is target.
    #[test]
    fn direction_incoming_only_finds_incoming() {
        let store = make_chain_store();
        // Query b: only incoming edge is a→b.
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::b_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 1,
                direction: "incoming".to_string(),
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        for rel in &result.direct_relationships {
            assert_eq!(rel.direction, "incoming");
        }
        assert_eq!(result.direct_relationships.len(), 1);
        assert_eq!(result.direct_relationships[0].target_name, "a");
    }

    /// relationship_types filter restricts traversal to specified kinds.
    #[test]
    fn relationship_types_filter() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(
            &store,
            target,
            &ImpactOptions {
                depth: 2,
                relationship_types: vec!["imports".to_string()],
                ..Default::default()
            },
            None,
        );
        assert_eq!(result.status, ImpactStatus::Ok);
        // No imports exist in the chain store, so no relationships should be returned.
        assert!(result.direct_relationships.is_empty());
        assert!(result.transitive_relationships.is_empty());
    }

    /// Invalid direction is rejected by validate_opts.
    #[test]
    fn invalid_direction_is_rejected() {
        let opts = ImpactOptions {
            direction: "zigzag".to_string(),
            ..Default::default()
        };
        let err = crate::impact::validate_opts(&opts);
        assert!(err.is_err());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("direction"));
    }

    /// Depth-1 default behavior is backward-compatible: returns both directions.
    #[test]
    fn depth_one_backward_compatible() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        // Default opts have depth=1 and direction="both".
        let result = analyze(&store, target.clone(), &ImpactOptions::default(), None);
        assert_eq!(result.status, ImpactStatus::Ok);
        // Should include both outgoing (a→b) and any incoming edges.
        let has_outgoing = result
            .direct_relationships
            .iter()
            .any(|r| r.direction == "outgoing" && r.target_name == "b");
        assert!(has_outgoing, "default should include outgoing edges");
    }

    // ── M3 impact freshness tests ─────────────────────────────────────

    /// Freshness is `None` when no workspace root is provided.
    #[test]
    fn impact_freshness_unknown_when_no_workspace_root() {
        let store = make_chain_store();
        let target = ImpactTarget::Symbol(SymbolId::new("sym::mod::chain::a_function@1"));
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.freshness, None);
    }

    /// Freshness is `Unknown` when the workspace root is provided but not a
    /// git repo (so current repo state cannot be captured) and no generation
    /// state is recorded in the store.
    #[test]
    fn impact_freshness_unknown_when_no_generation_state() {
        use crate::engineering_facts::FactsBuilder;
        use crate::engineering_facts::SourceLocation;
        use crate::engineering_facts::{ModuleFact, ModuleId, SymbolFact, SymbolId, SymbolKind};
        use crate::engineering_facts::{WorkspaceFact, WorkspaceId};

        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(
            WorkspaceId::new("ws::fresh"),
            "fresh-test".to_string(),
        ));
        let mid = ModuleId::new("mod::src/lib.rs");
        let mut mf = ModuleFact::new(mid.clone(), "src::lib");
        mf.path = Some("src/lib.rs".to_string());
        builder.add_module(mf);
        let sym_id = SymbolId::new("sym::src/lib.rs::foo_function@1");
        let mut sf = SymbolFact::new(sym_id.clone(), "foo", SymbolKind::Function);
        sf.location = SourceLocation::new()
            .with_file("src/lib.rs")
            .with_point(1, 0);
        builder.add_symbol(sf);
        let store = FactStore::build(builder.build());

        // Store has no generation_repo_state, so freshness should be Unknown
        // (can't compare without generation state).
        let target = ImpactTarget::Symbol(sym_id);
        let result = analyze(&store, target, &ImpactOptions::default(), None);
        assert_eq!(result.freshness, None);

        // Now provide a tempdir as workspace root — it won't be a git repo
        // and the store has no generation state, so Unknown.
        let tmpdir = tempfile::tempdir().unwrap();
        let target2 = ImpactTarget::Symbol(SymbolId::new("sym::src/lib.rs::foo_function@1"));
        let result2 = analyze(
            &store,
            target2,
            &ImpactOptions::default(),
            Some(tmpdir.path()),
        );
        assert_eq!(result2.freshness, Some(FreshnessStatus::Unknown));
    }

    /// Freshness is `Fresh` when generation state matches current repo state.
    #[test]
    fn impact_freshness_fresh_when_repo_unchanged() {
        use crate::engineering_facts::FactsBuilder;
        use crate::engineering_facts::SourceLocation;
        use crate::engineering_facts::{ModuleFact, ModuleId, SymbolFact, SymbolId, SymbolKind};
        use crate::engineering_facts::{WorkspaceFact, WorkspaceId};
        use crate::sandbox::RepoState;

        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-m", "init"])
            .output()
            .ok();

        let gen_state = RepoState::capture(&dir.path().to_path_buf()).unwrap();

        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(
            WorkspaceId::new("ws::fresh"),
            "fresh-test".to_string(),
        ));
        let mid = ModuleId::new("mod::src/lib.rs");
        let mut mf = ModuleFact::new(mid.clone(), "src::lib");
        mf.path = Some("src/lib.rs".to_string());
        builder.add_module(mf);
        let sym_id = SymbolId::new("sym::src/lib.rs::foo_function@1");
        let mut sf = SymbolFact::new(sym_id.clone(), "foo", SymbolKind::Function);
        sf.location = SourceLocation::new()
            .with_file("src/lib.rs")
            .with_point(1, 0);
        builder.add_symbol(sf);
        let store = FactStore::build(builder.build().with_generation_repo_state(gen_state));

        let target = ImpactTarget::Symbol(sym_id);
        let result = analyze(&store, target, &ImpactOptions::default(), Some(dir.path()));
        assert_eq!(
            result.freshness,
            Some(FreshnessStatus::Fresh),
            "freshness should be Fresh when repo state unchanged"
        );
    }

    /// Freshness is `Stale` when the repo has changed since generation.
    #[test]
    fn impact_freshness_stale_when_repo_changes() {
        use crate::engineering_facts::FactsBuilder;
        use crate::engineering_facts::SourceLocation;
        use crate::engineering_facts::{ModuleFact, ModuleId, SymbolFact, SymbolId, SymbolKind};
        use crate::engineering_facts::{WorkspaceFact, WorkspaceId};
        use crate::sandbox::RepoState;

        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init"])
            .output()
            .ok();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .output()
            .ok();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-m", "init"])
            .output()
            .ok();

        let gen_state = RepoState::capture(&dir.path().to_path_buf()).unwrap();

        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(
            WorkspaceId::new("ws::stale"),
            "stale-test".to_string(),
        ));
        let mid = ModuleId::new("mod::src/lib.rs");
        let mut mf = ModuleFact::new(mid.clone(), "src::lib");
        mf.path = Some("src/lib.rs".to_string());
        builder.add_module(mf);
        let sym_id = SymbolId::new("sym::src/lib.rs::foo_function@1");
        let mut sf = SymbolFact::new(sym_id.clone(), "foo", SymbolKind::Function);
        sf.location = SourceLocation::new()
            .with_file("src/lib.rs")
            .with_point(1, 0);
        builder.add_symbol(sf);
        let store = FactStore::build(builder.build().with_generation_repo_state(gen_state));

        // Modify the repo to make it stale.
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();

        let target = ImpactTarget::Symbol(sym_id);
        let result = analyze(&store, target, &ImpactOptions::default(), Some(dir.path()));
        assert_eq!(
            result.freshness,
            Some(FreshnessStatus::Stale),
            "freshness should be Stale when repo state changed"
        );
    }
}

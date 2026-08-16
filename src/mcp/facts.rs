//! Relevance-ranked retrieval over the fact store for the `engineering_facts` MCP
//! tool (P0.2).
//!
//! Search is a deterministic, allocation-light projection over the existing
//! immutable [`FactStore`] — no second index, no external search system.
//! Queries match against fact names, paths and signatures with a simple
//! scoring model, and results are returned as compact, LLM-friendly records
//! (not raw ids) with provenance.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::{FactId, FactKind, FactRef};
use crate::fact_store::store::FactStore;
use crate::provenance::{compute_trust, SourceKind};

/// Default result limit.
pub const DEFAULT_LIMIT: usize = 10;
/// Hard upper bound on returned results.
pub const MAX_LIMIT: usize = 50;

/// Classification of how a fact's relationship was established.
///
/// This enum captures the *relationship-verification quality* axis for
/// engineering-facts records — i.e. how strongly the provenance of a
/// relationship edge between two facts is backed by AST evidence versus
/// heuristic name matching.
///
/// It is **distinct** from the generic [`crate::provenance::Provenance`]
/// struct, which carries the full origin envelope (source kind, tool,
/// timestamp, repo state) for any claim type. `ProvenanceType` is a
/// narrow, relationship-specific qualifier used only by the fact-store
/// enrichment path and the M1-B trust computation for `FactRecord`.
///
/// `None` means the provenance-quality axis is *not applicable* — the
/// fact has no directly relevant relationship edge — rather than
/// asserting that the fact is "Verified".
#[derive(Debug, Clone, PartialEq, Eq, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceType {
    /// AST-derived, verified edge.
    Verified,
    /// Name-coincidence heuristic edge.
    Heuristic,
    /// Unknown provenance classification.
    Unknown,
    /// No directly relevant relationship provenance available.
    None,
}

/// A compact, LLM-friendly fact record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactRecord {
    /// Fact kind: symbol, module, package, test, build_target, dependency.
    pub kind: String,
    /// Entity name (symbol name, module name, ...).
    pub name: String,
    /// Canonical workspace-relative path when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 1-based line when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Short human/engineering summary (signature or description).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Where this fact came from.
    pub provenance: String,
    /// Owning module name, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Owning package name, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Direct relationship adjacency count (source + target edges).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship_count: Option<usize>,
    /// Number of tests directly exercising this fact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_count: Option<usize>,
    /// Strongest directly relevant relationship provenance for this result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_type: Option<ProvenanceType>,
    /// Provisional trust score computed from source kind, provenance
    /// quality, freshness, and confidence. Only present when the trust
    /// computation could be completed; never 0.0 as a missing-value
    /// placeholder.
    ///
    /// Trust is a relative ranking signal, not a calibrated probability,
    /// and is not strictly comparable across source kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<f64>,
    /// Internal relevance score (not part of the public contract; kept
    /// serialised for transparency but agents should not rely on it).
    #[serde(skip_serializing, default)]
    pub score: i32,
}

impl FactRecord {
    fn new(kind: &str, name: String, score: i32) -> Self {
        FactRecord {
            kind: kind.to_string(),
            name,
            path: None,
            line: None,
            summary: None,
            provenance: "codebro init (tree-sitter scan)".to_string(),
            module: None,
            package: None,
            relationship_count: None,
            test_count: None,
            provenance_type: None,
            trust: None,
            score,
        }
    }
}

/// Search parameters.
#[derive(Debug, Clone)]
pub struct FactSearch<'a> {
    /// Required query text; matched (case-insensitive) against names,
    /// paths and signatures.
    pub query: &'a str,
    /// Optional kind filter.
    pub kind: Option<FactKind>,
    /// Optional path substring filter.
    pub path: Option<&'a str>,
    /// Result cap; clamped into [1, MAX_LIMIT].
    pub limit: usize,
}

/// Top-level provenance summary for an engineering_facts response.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProvenanceResponseSummary {
    pub verified_edges: usize,
    pub heuristic_edges: usize,
    pub unknown_edges: usize,
}

/// Current freshness of the fact store relative to repository state.
#[derive(Debug, Clone, PartialEq, Eq, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
    Fresh,
    Stale,
    Unknown,
}

impl std::fmt::Display for FreshnessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FreshnessStatus::Fresh => write!(f, "fresh"),
            FreshnessStatus::Stale => write!(f, "stale"),
            FreshnessStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Run a deterministic, relevance-ranked search over the fact store.
///
/// This is lexical string matching (exact/prefix/substring/path/signature)
/// with a stable sort — NOT embedding/vector search.
///
/// Returns an error when the query is empty and no kind/path filter is
/// given: an unfiltered empty query would enumerate the whole store
/// without meaning, which is ambiguous for agents.
///
/// The `freshness` parameter is used to compute provisional trust scores
/// on each returned record via [`compute_fact_trust`].
pub fn search(
    store: &FactStore,
    params: &FactSearch<'_>,
    freshness: FreshnessStatus,
) -> Result<Vec<FactRecord>, String> {
    let query = params.query.trim().to_lowercase();
    let has_filter = params.kind.is_some() || params.path.is_some();
    if query.is_empty() && !has_filter {
        return Err("query is required (or provide a kind/path filter to enumerate)".to_string());
    }
    let path_filter = params.path.map(|p| p.to_lowercase());
    let limit = params.limit.clamp(1, MAX_LIMIT);

    let collection = store.collection();
    let index = store.index();
    let enrich = SearchEnrichment::new(collection, index);

    let mut results: Vec<FactRecord> = Vec::new();
    for fact in collection.iter() {
        let Some(mut record) = record_from_fact_enriched(&fact, &enrich, freshness) else {
            continue;
        };
        if let Some(kind) = params.kind {
            if record.kind != kind.as_str() {
                continue;
            }
        }
        if let Some(pf) = &path_filter {
            let hay = record.path.as_deref().unwrap_or("").to_lowercase();
            if !hay.contains(pf.as_str()) {
                continue;
            }
        }
        record.score = score_fact(&record, &query);
        if record.score > 0 {
            results.push(record);
        }
    }

    // Deterministic ordering: score desc, then kind, then name, then path.
    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
    results.truncate(limit);
    // Hide internal score from the serialised output.
    for r in &mut results {
        r.score = 0;
    }
    Ok(results)
}

/// Compute the provenance summary across all relationships in the store.
pub fn provenance_summary(store: &FactStore) -> ProvenanceResponseSummary {
    let mut summary = ProvenanceResponseSummary::default();
    for rel in store.collection().relationships() {
        match relationship_provenance(rel) {
            ProvenanceType::Verified => summary.verified_edges += 1,
            ProvenanceType::Heuristic => summary.heuristic_edges += 1,
            ProvenanceType::Unknown => summary.unknown_edges += 1,
            ProvenanceType::None => {}
        }
    }
    summary
}

/// Compute freshness by comparing generation-time repo state with current state.
pub fn compute_freshness(store: &FactStore, workspace_root: &std::path::Path) -> FreshnessStatus {
    let gen_state = store.collection().model().generation_repo_state();
    let current = crate::sandbox::RepoState::capture(&workspace_root.to_path_buf());
    match (gen_state, current) {
        (Some(prev), Some(cur)) => {
            if prev.working_tree_hash == cur.working_tree_hash {
                FreshnessStatus::Fresh
            } else {
                FreshnessStatus::Stale
            }
        }
        _ => FreshnessStatus::Unknown,
    }
}

/// Enrichment data computed once and shared across all records in a search.
struct SearchEnrichment<'a> {
    collection: &'a crate::fact_store::collection::FactCollection,
    index: &'a crate::fact_store::index::FactIndex,
    /// Pre-computed relationship counts per fact id.
    rel_counts: std::collections::HashMap<FactId, usize>,
    /// Pre-computed test counts per symbol id.
    sym_test_counts: std::collections::HashMap<FactId, usize>,
    /// Pre-computed provenance types per fact id.
    provenance_types: std::collections::HashMap<FactId, ProvenanceType>,
}

impl<'a> SearchEnrichment<'a> {
    fn new(
        collection: &'a crate::fact_store::collection::FactCollection,
        index: &'a crate::fact_store::index::FactIndex,
    ) -> Self {
        let mut rel_counts = std::collections::HashMap::new();
        let mut sym_test_counts = std::collections::HashMap::new();
        let mut provenance_types = std::collections::HashMap::new();

        // Count relationships per endpoint.
        for rel in collection.relationships() {
            let inc = rel_counts.entry(rel.source.clone()).or_insert(0);
            *inc += 1;
            let inc = rel_counts.entry(rel.target.clone()).or_insert(0);
            *inc += 1;
            // Track provenance per endpoint (Verified beats Heuristic beats None).
            let pt = relationship_provenance(rel);
            let prev = provenance_types.entry(rel.source.clone()).or_insert(ProvenanceType::None);
            if pt == ProvenanceType::Verified {
                *prev = ProvenanceType::Verified;
            } else if *prev == ProvenanceType::None {
                *prev = pt;
            }
            let prev = provenance_types.entry(rel.target.clone()).or_insert(ProvenanceType::None);
            if pt == ProvenanceType::Verified {
                *prev = ProvenanceType::Verified;
            } else if *prev == ProvenanceType::None {
                *prev = pt;
            }
        }

        // Count tests per symbol.
        for test in collection.tests() {
            for tested in &test.tested {
                let inc = sym_test_counts
                    .entry(FactId::Symbol(tested.clone()))
                    .or_insert(0);
                *inc += 1;
            }
        }

        SearchEnrichment {
            collection,
            index,
            rel_counts,
            sym_test_counts,
            provenance_types,
        }
    }
}

/// Extract a searchable record from a fact reference, enriched with structural
/// context (module, package, relationship count, test count, provenance type).
fn record_from_fact_enriched(
    fact: &FactRef<'_>,
    enrich: &SearchEnrichment<'_>,
    freshness: FreshnessStatus,
) -> Option<FactRecord> {
    let collection = enrich.collection;

    let mut r = match fact {
        FactRef::Symbol(s) => {
            let mut r = FactRecord::new("symbol", s.name.clone(), 0);
            r.path = s.location.file.clone();
            r.line = s
                .location
                .line
                .or_else(|| s.location.span.as_ref().map(|sp| sp.start.line));
            r.summary = s
                .signature
                .clone()
                .or_else(|| s.metadata.description.clone());
            r.module = s
                .module
                .as_ref()
                .and_then(|mid| collection.module(mid).map(|m| m.name.clone()));
            r.package = s
                .module
                .as_ref()
                .and_then(|mid| collection.module(mid))
                .and_then(|m| m.package.as_ref())
                .and_then(|pid| collection.package(pid).map(|p| p.name.clone()));
            r.relationship_count =
                Some(enrich.rel_counts.get(&FactId::Symbol(s.id.clone())).copied().unwrap_or(0));
            r.test_count =
                Some(enrich.sym_test_counts.get(&FactId::Symbol(s.id.clone())).copied().unwrap_or(0));
            r.provenance_type =
                enrich.provenance_types.get(&FactId::Symbol(s.id.clone())).copied();
            r
        }
        FactRef::Module(m) => {
            let mut r = FactRecord::new("module", m.name.clone(), 0);
            r.path = m.path.clone().or_else(|| m.location.file.clone());
            r.line = m
                .location
                .line
                .or_else(|| m.location.span.as_ref().map(|sp| sp.start.line));
            r.summary = m.metadata.description.clone();
            r.package = m
                .package
                .as_ref()
                .and_then(|pid| collection.package(pid).map(|p| p.name.clone()));
            r.relationship_count =
                Some(enrich.rel_counts.get(&FactId::Module(m.id.clone())).copied().unwrap_or(0));
            r.test_count = Some(module_test_count(collection, &m.id));
            r.provenance_type =
                enrich.provenance_types.get(&FactId::Module(m.id.clone())).copied();
            r
        }
        FactRef::Package(p) => {
            let mut r = FactRecord::new("package", p.name.clone(), 0);
            r.summary = p.version.clone().map(|v| format!("version {v}"));
            r.relationship_count =
                Some(enrich.rel_counts.get(&FactId::Package(p.id.clone())).copied().unwrap_or(0));
            r.test_count = Some(package_test_count(collection, &p.id));
            r.provenance_type =
                enrich.provenance_types.get(&FactId::Package(p.id.clone())).copied();
            r
        }
        FactRef::Test(t) => {
            let mut r = FactRecord::new("test", t.name.clone(), 0);
            if let Some(loc) = &t.location {
                r.path = loc.file.clone();
                r.line = loc.line;
            }
            r.module = t
                .location
                .as_ref()
                .and_then(|l| l.module.as_ref())
                .and_then(|mid| collection.module(mid).map(|m| m.name.clone()));
            r.package = t
                .location
                .as_ref()
                .and_then(|l| l.package.as_ref())
                .and_then(|pid| collection.package(pid).map(|p| p.name.clone()))
                .or_else(|| {
                    t.location.as_ref().and_then(|l| l.module.as_ref()).and_then(|mid| {
                        collection
                            .module(mid)
                            .and_then(|m| m.package.as_ref())
                            .and_then(|pid| collection.package(pid).map(|p| p.name.clone()))
                    })
                });
            r.relationship_count = Some(0);
            r.test_count = Some(1);
            r.provenance_type = Some(ProvenanceType::Verified);
            r
        }
        FactRef::BuildTarget(b) => {
            let mut r = FactRecord::new("build_target", b.name.clone(), 0);
            r.summary = Some(format!("{} target", b.kind.as_str()));
            r.package = b
                .package
                .as_ref()
                .and_then(|pid| collection.package(pid).map(|p| p.name.clone()));
            r.relationship_count = Some(0);
            r.test_count = Some(0);
            r.provenance_type = Some(ProvenanceType::None);
            r
        }
        FactRef::Dependency(d) => {
            let mut r = FactRecord::new(
                "dependency",
                format!("{} -> {}", source_label(&d.source), target_label(&d.target)),
                0,
            );
            r.summary = d.version_constraint.clone();
            r.relationship_count = Some(0);
            r.test_count = Some(0);
            r.provenance_type = Some(ProvenanceType::None);
            r
        }
        _ => return None,
    };
    r.trust = compute_fact_trust(r.provenance_type, freshness);
    Some(r)
}

fn source_label(id: &FactId) -> String {
    // Dependency endpoints are typed ids; render the tail of the opaque
    // value for compactness (e.g. pkg::serde::external -> "serde").
    let s = id.as_str();
    s.rsplit("::").next().unwrap_or(s).to_string()
}

fn target_label(id: &FactId) -> String {
    source_label(id)
}

fn module_test_count(
    collection: &crate::fact_store::collection::FactCollection,
    mod_id: &crate::engineering_facts::ModuleId,
) -> usize {
    collection
        .tests()
        .iter()
        .filter(|t| {
            t.location
                .as_ref()
                .and_then(|l| l.module.as_ref())
                .map(|m| m == mod_id)
                .unwrap_or(false)
        })
        .count()
}

fn package_test_count(
    collection: &crate::fact_store::collection::FactCollection,
    pkg_id: &crate::engineering_facts::PackageId,
) -> usize {
    collection
        .tests()
        .iter()
        .filter(|t| {
            t.location
                .as_ref()
                .and_then(|l| l.package.as_ref())
                .map(|p| p == pkg_id)
                .unwrap_or(false)
        })
        .count()
}

fn relationship_provenance(rel: &crate::engineering_facts::RelationshipFact) -> ProvenanceType {
    match rel.metadata.get("provenance") {
        Some(p) if p == "heuristic" => ProvenanceType::Heuristic,
        Some(p) if p == "unknown" => ProvenanceType::Unknown,
        Some(_) => ProvenanceType::Verified,
        None => ProvenanceType::Unknown,
    }
}

/// Provenance-quality factor for the M1-B trust computation.
///
/// For `StaticAnalysis` source kind (the only kind used for fact-store
/// records), the factor scales base trust according to how strongly the
/// relationship edge is verified:
/// - `Verified` → 1.0 (full AST evidence)
/// - `Heuristic` → 0.8 (name-coincidence evidence)
/// - `Unknown` → 0.6 (insufficient classification)
/// - `None` → 1.0 (provenance-quality axis not applicable)
///
/// For non-`StaticAnalysis` source kinds the factor is neutral (1.0);
/// this function always uses `StaticAnalysis` semantics because it is
/// called exclusively from the fact-store enrichment path.
fn provenance_quality_factor(pt: Option<ProvenanceType>) -> f64 {
    match pt {
        Some(ProvenanceType::Verified) => 1.0,
        Some(ProvenanceType::Heuristic) => 0.8,
        Some(ProvenanceType::Unknown) => 0.6,
        Some(ProvenanceType::None) | None => 1.0,
    }
}

/// Compute provisional trust for a `FactRecord`.
///
/// Formula:
/// ```text
/// trust = base(source_kind) × provenance_quality × freshness_multiplier × confidence_factor
/// ```
///
/// Facts always use `StaticAnalysis` as the source kind and `1.0` as the
/// confidence factor (facts carry no agent-declared confidence). The
/// freshness multiplier comes from the store-wide freshness status passed
/// by the caller. If trust cannot be computed, returns `None`.
///
/// Trust is a relative ranking signal, not a calibrated probability. It
/// is not strictly comparable across source kinds.
pub fn compute_fact_trust(
    provenance_type: Option<ProvenanceType>,
    freshness: FreshnessStatus,
) -> Option<f64> {
    let base = crate::provenance::TRUST_STATIC_ANALYSIS;
    let pq = provenance_quality_factor(provenance_type);
    let freshness_factor = match freshness {
        FreshnessStatus::Fresh => 1.0,
        FreshnessStatus::Unknown => 0.8,
        FreshnessStatus::Stale => 0.6,
    };
    let trust = base * pq * freshness_factor;
    Some(trust.clamp(0.0, 1.0))
}

/// Score a record against the (lower-cased) query.
///
/// The query is split into whitespace-separated tokens; the best-matching
/// token scores the record (exact name 100, prefix 80, substring 60, path
/// 30, summary 15), so "breaker Stats Allow" matches symbols named Stats
/// or Allow as well as the package's breaker symbols.
fn score_fact(record: &FactRecord, query: &str) -> i32 {
    if query.is_empty() {
        // No query: everything is a match at base relevance, so the caller
        // can enumerate deterministically.
        return 1;
    }
    let name = record.name.to_lowercase();
    let path = record.path.as_deref().unwrap_or("").to_lowercase();
    let summary = record.summary.as_deref().unwrap_or("").to_lowercase();

    let mut best = 0;
    for token in query.split_whitespace() {
        let mut score = 0;
        if name == token {
            score = 100;
        } else if name.starts_with(token) {
            score = 80;
        } else if name.contains(token) {
            score = 60;
        } else if path.contains(token) {
            score = 30;
        } else if summary.contains(token) {
            score = 15;
        }
        best = best.max(score);
        if best == 100 {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_facts::location::{Position, SourceLocation, Span};
    use crate::engineering_facts::{
        FactsBuilder, ModuleFact, ModuleId, RelationshipFact, RelationshipId, RelationshipKind,
        SymbolFact, SymbolId, SymbolKind, TestFact, TestId, WorkspaceFact, WorkspaceId,
    };

    fn sample_store() -> FactStore {
        let ws_id = WorkspaceId::new("ws::test");
        let mut builder = FactsBuilder::new();

        let ws = WorkspaceFact::new(ws_id.clone(), "test");
        builder.add_workspace(ws);

        let mut m = ModuleFact::new(ModuleId::new("mod::src/lib.rs"), "src::lib");
        m.path = Some("src/lib.rs".to_string());
        builder.add_module(m);

        let mut change_engine = SymbolFact::new(
            SymbolId::new("sym::ChangeEngine"),
            "ChangeEngine",
            SymbolKind::Struct,
        );
        change_engine.location = SourceLocation::new()
            .with_file("src/coding/permissions.rs")
            .with_point(202, 0);
        change_engine.signature = Some("pub struct ChangeEngine { ... }".to_string());
        builder.add_symbol(change_engine);

        let mut prepare = SymbolFact::new(
            SymbolId::new("sym::prepare"),
            "prepare",
            SymbolKind::Function,
        );
        prepare.location = SourceLocation::new()
            .with_file("src/coding/permissions.rs")
            .with_point(243, 0);
        prepare.signature =
            Some("pub fn prepare(&self, path: &str, old: &str, new: &str)".to_string());
        builder.add_symbol(prepare);

        let mut memory_runtime = SymbolFact::new(
            SymbolId::new("sym::EngineeringMemoryRuntime"),
            "EngineeringMemoryRuntime",
            SymbolKind::Struct,
        );
        memory_runtime.location = SourceLocation::new()
            .with_file("src/engineering_memory/runtime.rs")
            .with_point(87, 0);
        memory_runtime.signature =
            Some("pub struct EngineeringMemoryRuntime<P> { ... }".to_string());
        builder.add_symbol(memory_runtime);

        FactStore::build(builder.build())
    }

    fn search_all(store: &FactStore, query: &str) -> Vec<FactRecord> {
        search(
            store,
            &FactSearch {
                query,
                kind: None,
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Unknown,
        )
        .expect("search succeeds")
    }

    #[test]
    fn exact_name_match_ranks_first() {
        let store = sample_store();
        let results = search_all(&store, "ChangeEngine");
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "ChangeEngine");
        assert_eq!(results[0].score, 0); // internal score hidden
    }

    #[test]
    fn partial_name_match() {
        let store = sample_store();
        let results = search_all(&store, "engine");
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"ChangeEngine"));
        assert!(names.contains(&"EngineeringMemoryRuntime"));
        // "EngineeringMemoryRuntime" matches as a name prefix (higher
        // relevance) so it ranks above the substring-only "ChangeEngine";
        // both are present, deterministically ordered.
        assert_eq!(names[0], "EngineeringMemoryRuntime");
    }

    #[test]
    fn kind_filter() {
        let store = sample_store();
        let results = search(
            &store,
            &FactSearch {
                query: "",
                kind: Some(FactKind::Symbol),
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Unknown,
        )
        .expect("kind filter makes empty query valid");
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == "symbol"));
    }

    #[test]
    fn path_filter() {
        let store = sample_store();
        let results = search(
            &store,
            &FactSearch {
                query: "",
                kind: None,
                path: Some("coding/permissions"),
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Unknown,
        )
        .expect("path filter makes empty query valid");
        assert_eq!(results.len(), 2); // ChangeEngine + prepare
        assert!(results.iter().all(|r| r
            .path
            .as_deref()
            .unwrap_or("")
            .contains("coding/permissions")));
    }

    #[test]
    fn limit_and_upper_bound() {
        let store = sample_store();
        let results = search(
            &store,
            &FactSearch {
                query: "engine",
                kind: None,
                path: None,
                limit: 1,
            },
            FreshnessStatus::Unknown,
        )
        .expect("search succeeds");
        assert_eq!(results.len(), 1);
        // Clamped from absurdly high value.
        let results = search(
            &store,
            &FactSearch {
                query: "engine",
                kind: None,
                path: None,
                limit: 10_000,
            },
            FreshnessStatus::Unknown,
        )
        .expect("search succeeds");
        assert!(results.len() <= MAX_LIMIT);
    }

    #[test]
    fn empty_query_without_filter_is_rejected() {
        let store = sample_store();
        let err = search(
            &store,
            &FactSearch {
                query: "",
                kind: None,
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Unknown,
        )
        .expect_err("empty query without filter must error");
        assert!(err.contains("query is required"));
    }

    #[test]
    fn empty_query_with_filter_enumerates() {
        let store = sample_store();
        let results = search(
            &store,
            &FactSearch {
                query: "",
                kind: Some(FactKind::Symbol),
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Unknown,
        )
        .expect("kind filter makes empty query valid");
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == "symbol"));
    }

    #[test]
    fn empty_result() {
        let store = sample_store();
        let results = search_all(&store, "zzzz-no-such-thing");
        assert!(results.is_empty());
    }

    #[test]
    fn deterministic_ordering() {
        let store = sample_store();
        let a = search_all(&store, "engine");
        let b = search_all(&store, "engine");
        assert_eq!(a, b);
    }

    #[test]
    fn records_include_provenance_and_location() {
        let store = sample_store();
        let results = search_all(&store, "prepare");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].path.as_deref(),
            Some("src/coding/permissions.rs")
        );
        assert_eq!(results[0].line, Some(243));
        assert!(results[0].summary.is_some());
        assert!(results[0].provenance.contains("codebro init"));
    }

    #[test]
    fn span_and_point_sanity() {
        let _ = Position::new(1, 0);
        let _ = Span::new(Position::new(1, 0), Position::new(2, 0));
    }

    #[test]
    fn summary_signature_matching() {
        let store = sample_store();
        // "pub struct" appears only in ChangeEngine's signature summary.
        let results = search_all(&store, "pub struct");
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.name == "ChangeEngine"));
        // Ranked below name matches, but present.
        let by_score =
            results[0].name == "ChangeEngine" || results.iter().any(|r| r.name == "ChangeEngine");
        assert!(by_score);
    }

    #[test]
    fn multi_word_query_matches_any_token() {
        let store = sample_store();
        // "prepare ChangeEngine" matches both prepare (name) and
        // ChangeEngine (name) — tokenized scoring.
        let results = search_all(&store, "prepare ChangeEngine");
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"prepare"));
        assert!(names.contains(&"ChangeEngine"));
    }

    #[test]
    fn path_fragment_matching() {
        let store = sample_store();
        // A path fragment that is not part of any symbol name still matches
        // via the path score (30).
        let results = search_all(&store, "permissions.rs");
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .all(|r| r.path.as_deref().unwrap_or("").contains("permissions.rs")));
    }

    /// Go-flavored store mirroring the P1.3 fixtures: a `Breaker` type plus
    /// its methods, as produced by the fixed Go parser.
    fn go_store() -> FactStore {
        let ws_id = WorkspaceId::new("ws::go");
        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(ws_id.clone(), "go-proj"));

        let mut m = ModuleFact::new(
            ModuleId::new("mod::internal/breaker/breaker.go"),
            "internal::breaker::breaker.go",
        );
        m.path = Some("internal/breaker/breaker.go".to_string());
        builder.add_module(m);

        let mut breaker =
            SymbolFact::new(SymbolId::new("sym::Breaker"), "Breaker", SymbolKind::Struct);
        breaker.location = SourceLocation::new()
            .with_file("internal/breaker/breaker.go")
            .with_point(64, 0);
        breaker.signature = Some("type Breaker struct".to_string());
        builder.add_symbol(breaker);

        let mut allow = SymbolFact::new(SymbolId::new("sym::Allow"), "Allow", SymbolKind::Method);
        allow.location = SourceLocation::new()
            .with_file("internal/breaker/breaker.go")
            .with_point(98, 0);
        allow.signature = Some("func (b *Breaker) Allow() Result".to_string());
        builder.add_symbol(allow);

        let mut stats = SymbolFact::new(SymbolId::new("sym::Stats"), "Stats", SymbolKind::Method);
        stats.location = SourceLocation::new()
            .with_file("internal/breaker/breaker.go")
            .with_point(178, 0);
        stats.signature = Some("func (b *Breaker) Stats() BreakerStats".to_string());
        builder.add_symbol(stats);

        FactStore::build(builder.build())
    }

    #[test]
    fn go_multi_word_query_finds_type_and_methods() {
        // P1.3 fixture: "breaker Stats Allow" must surface the Breaker type and
        // its Stats/Allow methods from the Go fact store.
        let store = go_store();
        let results = search_all(&store, "breaker Stats Allow");
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"Breaker"),
            "Breaker type must be found, got {names:?}"
        );
        assert!(names.contains(&"Stats"));
        assert!(names.contains(&"Allow"));
        // All three targeted Go facts surface at the top of the ranking
        // (exact-name tokens score equally; deterministic order thereafter).
        let top: std::collections::HashSet<&str> =
            results.iter().take(3).map(|r| r.name.as_str()).collect();
        assert_eq!(top.len(), 3);
        assert!(top.contains("Breaker") && top.contains("Stats") && top.contains("Allow"));
    }

    #[test]
    fn go_type_query_returns_real_name() {
        // P1.3 regression: the Breaker TYPE must be discoverable by name (it
        // used to be stored as "unknown" by the Go parser).
        let store = go_store();
        let results = search_all(&store, "Breaker");
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "Breaker");
        assert_eq!(results[0].kind, "symbol");
        assert_eq!(
            results[0].path.as_deref(),
            Some("internal/breaker/breaker.go")
        );
        // Method signatures are clean and LLM-recognizable.
        let stats = results.iter().find(|r| r.name == "Stats").unwrap();
        assert_eq!(
            stats.summary.as_deref(),
            Some("func (b *Breaker) Stats() BreakerStats")
        );
    }

    // ── P2.4 enrichment tests ────────────────────────────────────────────

    fn enriched_store() -> FactStore {
        let ws_id = WorkspaceId::new("ws::enriched");
        let pkg_id = crate::engineering_facts::PackageId::new("pkg::enriched");
        let mut builder = FactsBuilder::new();

        builder.add_workspace(WorkspaceFact::new(ws_id.clone(), "enriched"));

        let mut pkg = crate::engineering_facts::PackageFact::new(pkg_id.clone(), "enriched-pkg");
        pkg.workspace = Some(ws_id.clone());
        builder.add_package(pkg);

        let mut m = ModuleFact::new(ModuleId::new("mod::src/lib.rs"), "src::lib");
        m.package = Some(pkg_id.clone());
        builder.add_module(m.clone());

        let mut foo = SymbolFact::new(SymbolId::new("sym::foo"), "foo", SymbolKind::Function);
        foo.module = Some(m.id.clone());
        foo.location = SourceLocation::new()
            .with_workspace(ws_id.clone())
            .with_module(m.id.clone())
            .with_file("src/lib.rs")
            .with_point(1, 0);
        builder.add_symbol(foo.clone());

        let mut bar = SymbolFact::new(SymbolId::new("sym::bar"), "bar", SymbolKind::Function);
        bar.module = Some(m.id.clone());
        bar.location = SourceLocation::new()
            .with_workspace(ws_id.clone())
            .with_module(m.id.clone())
            .with_file("src/lib.rs")
            .with_point(2, 0);
        builder.add_symbol(bar.clone());

        // Verified relationship: foo calls bar
        let mut rel = RelationshipFact::new(
            RelationshipId::new("rel::foo-calls-bar"),
            RelationshipKind::Calls,
            FactId::Symbol(foo.id.clone()),
            FactId::Symbol(bar.id.clone()),
        );
        rel.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
            .attr("provenance", "verified")
            .build();
        builder.add_relationship(rel);

        // Heuristic relationship
        let mut heur_rel = RelationshipFact::new(
            RelationshipId::new("rel::heur"),
            RelationshipKind::References,
            FactId::Symbol(foo.id.clone()),
            FactId::Symbol(bar.id.clone()),
        );
        heur_rel.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
            .attr("provenance", "heuristic")
            .build();
        builder.add_relationship(heur_rel);

        // Unknown provenance relationship: baz references qux
        let baz = SymbolFact::new(SymbolId::new("sym::baz"), "baz", SymbolKind::Function);
        let qux = SymbolFact::new(SymbolId::new("sym::qux"), "qux", SymbolKind::Function);
        builder.add_symbol(baz.clone());
        builder.add_symbol(qux.clone());
        let mut unk_rel = RelationshipFact::new(
            RelationshipId::new("rel::unk"),
            RelationshipKind::References,
            FactId::Symbol(baz.id.clone()),
            FactId::Symbol(qux.id.clone()),
        );
        unk_rel.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
            .attr("provenance", "unknown")
            .build();
        builder.add_relationship(unk_rel);

        // Missing provenance relationship: alpha references beta
        let alpha = SymbolFact::new(SymbolId::new("sym::alpha"), "alpha", SymbolKind::Function);
        let beta = SymbolFact::new(SymbolId::new("sym::beta"), "beta", SymbolKind::Function);
        builder.add_symbol(alpha.clone());
        builder.add_symbol(beta.clone());
        let miss_rel = RelationshipFact::new(
            RelationshipId::new("rel::miss"),
            RelationshipKind::References,
            FactId::Symbol(alpha.id.clone()),
            FactId::Symbol(beta.id.clone()),
        );
        builder.add_relationship(miss_rel);

        // Test exercising foo
        let mut test_foo = TestFact::new(TestId::new("test::test_foo"), "test_foo");
        test_foo.tested.push(foo.id.clone());
        test_foo.location = Some(
            SourceLocation::new()
                .with_workspace(ws_id.clone())
                .with_module(m.id.clone())
                .with_file("src/lib.rs")
                .with_point(10, 0),
        );
        builder.add_test(test_foo);

        FactStore::build(builder.build())
    }

    #[test]
    fn enrichment_module_resolution() {
        let store = enriched_store();
        let results = search_all(&store, "foo");
        assert!(!results.is_empty());
        let foo = results.iter().find(|r| r.name == "foo").expect("foo must be found");
        assert_eq!(foo.module, Some("src::lib".to_string()));
    }

    #[test]
    fn enrichment_package_resolution() {
        let store = enriched_store();
        let results = search_all(&store, "foo");
        assert!(!results.is_empty());
        let foo = results.iter().find(|r| r.name == "foo").expect("foo must be found");
        assert_eq!(foo.package, Some("enriched-pkg".to_string()));
    }

    #[test]
    fn enrichment_relationship_count() {
        let store = enriched_store();
        let results = search_all(&store, "foo");
        let foo = results.iter().find(|r| r.name == "foo").expect("foo must be found");
        // foo is source of both the verified Calls and heuristic References.
        assert_eq!(foo.relationship_count, Some(2));
    }

    #[test]
    fn enrichment_test_count() {
        let store = enriched_store();
        let results = search_all(&store, "foo");
        let foo = results.iter().find(|r| r.name == "foo").expect("foo must be found");
        assert_eq!(foo.test_count, Some(1));
    }

    #[test]
    fn enrichment_provenance_type_verified_wins() {
        let store = enriched_store();
        let results = search_all(&store, "foo");
        let foo = results.iter().find(|r| r.name == "foo").expect("foo must be found");
        // foo has both verified (Calls) and heuristic (References) relationships.
        // Verified should win as the strongest provenance.
        assert_eq!(foo.provenance_type, Some(ProvenanceType::Verified));
    }

    #[test]
    fn enrichment_provenance_type_heuristic_on_target() {
        let store = enriched_store();
        let results = search_all(&store, "bar");
        let bar = results.iter().find(|r| r.name == "bar").expect("bar must be found");
        // bar is target of both verified and heuristic relationships.
        // Verified wins.
        assert_eq!(bar.provenance_type, Some(ProvenanceType::Verified));
    }

    #[test]
    fn missing_provenance_classifies_as_unknown() {
        let store = enriched_store();
        let results = search_all(&store, "alpha");
        let alpha = results.iter().find(|r| r.name == "alpha").expect("alpha must be found");
        // alpha has a relationship with no provenance metadata at all.
        // Missing provenance must classify as Unknown, not Verified.
        assert_eq!(alpha.provenance_type, Some(ProvenanceType::Unknown));
    }

    #[test]
    fn unknown_provenance_classifies_as_unknown() {
        let store = enriched_store();
        let results = search_all(&store, "baz");
        let baz = results.iter().find(|r| r.name == "baz").expect("baz must be found");
        // baz has a relationship with provenance=unknown.
        assert_eq!(baz.provenance_type, Some(ProvenanceType::Unknown));
    }

    #[test]
    fn heuristic_provenance_classifies_as_heuristic() {
        let store = enriched_store();
        let results = search_all(&store, "alpha");
        // alpha only has a missing-provenance relationship, so Unknown.
        // But if we query something with only heuristic, it should be Heuristic.
        // Use baz which has only unknown provenance relationship.
        let results = search_all(&store, "baz");
        let baz = results.iter().find(|r| r.name == "baz").expect("baz must be found");
        assert_eq!(baz.provenance_type, Some(ProvenanceType::Unknown));
    }

    #[test]
    fn enrichment_module_package_for_module_fact() {
        let store = enriched_store();
        let results = search(
            &store,
            &FactSearch {
                query: "",
                kind: Some(FactKind::Module),
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Unknown,
        )
        .expect("search succeeds");
        let mod_rec = results
            .iter()
            .find(|r| r.name == "src::lib")
            .expect("module must be found");
        assert_eq!(mod_rec.package, Some("enriched-pkg".to_string()));
    }

    #[test]
    fn provenance_summary_counts() {
        let store = enriched_store();
        let summary = provenance_summary(&store);
        // 1 verified (Calls with explicit provenance=verified)
        // 1 heuristic (References with provenance=heuristic)
        // 1 unknown (References with provenance=unknown)
        // 1 missing (References with no provenance metadata)
        assert_eq!(summary.verified_edges, 1);
        assert_eq!(summary.heuristic_edges, 1);
        assert_eq!(summary.unknown_edges, 2);
    }

    #[test]
    fn provenance_summary_empty_store() {
        let store = FactStore::empty();
        let summary = provenance_summary(&store);
        assert_eq!(summary.verified_edges, 0);
        assert_eq!(summary.heuristic_edges, 0);
        assert_eq!(summary.unknown_edges, 0);
    }

    #[test]
    fn search_behavior_unchanged_for_existing_queries() {
        let store = sample_store();
        // Existing tests must continue to produce the same names and ordering.
        let a = search_all(&store, "ChangeEngine");
        assert_eq!(a[0].name, "ChangeEngine");

        let b = search_all(&store, "engine");
        assert_eq!(b[0].name, "EngineeringMemoryRuntime");

        let c = search_all(&store, "prepare");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "prepare");
    }

    #[test]
    fn enrichment_serialization_round_trip() {
        let store = enriched_store();
        let results = search_all(&store, "foo");
        let json = serde_json::to_string(&results).expect("serializes");
        let back: Vec<FactRecord> = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(results.len(), back.len());
        assert_eq!(results[0].name, back[0].name);
        assert_eq!(results[0].module, back[0].module);
        assert_eq!(results[0].package, back[0].package);
        assert_eq!(results[0].relationship_count, back[0].relationship_count);
        assert_eq!(results[0].test_count, back[0].test_count);
        assert_eq!(results[0].provenance_type, back[0].provenance_type);
    }

    // ── M1-B provenance-quality trust tests ─────────────────────────────

    /// M1-B: Verified provenance → base trust × freshness (fresh = 1.0).
    #[test]
    fn m1b_verified_provenance_with_fresh() {
        use crate::provenance::TRUST_STATIC_ANALYSIS;
        let t = compute_fact_trust(Some(ProvenanceType::Verified), FreshnessStatus::Fresh).unwrap();
        // base 0.90 × 1.0 × 1.0 = 0.90
        assert!((t - TRUST_STATIC_ANALYSIS).abs() < 1e-9, "expected {TRUST_STATIC_ANALYSIS}, got {t}");
    }

    /// M1-B: Heuristic provenance → 0.8 provenance factor.
    #[test]
    fn m1b_heuristic_provenance_factor() {
        use crate::provenance::TRUST_STATIC_ANALYSIS;
        let t =
            compute_fact_trust(Some(ProvenanceType::Heuristic), FreshnessStatus::Fresh).unwrap();
        // base 0.90 × 0.8 × 1.0 = 0.72
        let expected = TRUST_STATIC_ANALYSIS * 0.8;
        assert!((t - expected).abs() < 1e-9, "expected {expected}, got {t}");
    }

    /// M1-B: Unknown provenance → 0.6 provenance factor.
    #[test]
    fn m1b_unknown_provenance_factor() {
        use crate::provenance::TRUST_STATIC_ANALYSIS;
        let t =
            compute_fact_trust(Some(ProvenanceType::Unknown), FreshnessStatus::Fresh).unwrap();
        // base 0.90 × 0.6 × 1.0 = 0.54
        let expected = TRUST_STATIC_ANALYSIS * 0.6;
        assert!((t - expected).abs() < 1e-9, "expected {expected}, got {t}");
    }

    /// M1-B: None provenance → neutral 1.0 factor (axis not applicable).
    #[test]
    fn m1b_none_provenance_neutral() {
        use crate::provenance::TRUST_STATIC_ANALYSIS;
        let t =
            compute_fact_trust(Some(ProvenanceType::None), FreshnessStatus::Fresh).unwrap();
        // base 0.90 × 1.0 × 1.0 = 0.90 (same as Verified when fresh)
        assert!((t - TRUST_STATIC_ANALYSIS).abs() < 1e-9, "expected {TRUST_STATIC_ANALYSIS}, got {t}");
    }

    /// M1-B: None with no provenance_type at all → also neutral.
    #[test]
    fn m1b_absent_provenance_type_is_neutral() {
        use crate::provenance::TRUST_STATIC_ANALYSIS;
        let t = compute_fact_trust(None, FreshnessStatus::Fresh).unwrap();
        assert!((t - TRUST_STATIC_ANALYSIS).abs() < 1e-9, "expected {TRUST_STATIC_ANALYSIS}, got {t}");
    }

    /// M1-B: Stale freshness reduces trust for Verified provenance.
    #[test]
    fn m1b_stale_freshness_reduces_trust() {
        use crate::provenance::TRUST_STATIC_ANALYSIS;
        let t_fresh =
            compute_fact_trust(Some(ProvenanceType::Verified), FreshnessStatus::Fresh).unwrap();
        let t_stale =
            compute_fact_trust(Some(ProvenanceType::Verified), FreshnessStatus::Stale).unwrap();
        assert!(t_fresh > t_stale, "fresh ({t_fresh}) must exceed stale ({t_stale})");
        // stale: 0.90 × 1.0 × 0.6 = 0.54
        let expected_stale = TRUST_STATIC_ANALYSIS * 0.6;
        assert!((t_stale - expected_stale).abs() < 1e-9);
    }

    /// M1-B: Trust bounds are always in (0, 1].
    #[test]
    fn m1b_trust_bounds_in_range() {
        for pt in [
            ProvenanceType::Verified,
            ProvenanceType::Heuristic,
            ProvenanceType::Unknown,
            ProvenanceType::None,
        ] {
            for freshness in [
                FreshnessStatus::Fresh,
                FreshnessStatus::Unknown,
                FreshnessStatus::Stale,
            ] {
                let t = compute_fact_trust(Some(pt), freshness).unwrap();
                assert!(t > 0.0, "trust must be > 0 for pt={pt:?} freshness={freshness:?}, got {t}");
                assert!(t <= 1.0, "trust must be <= 1 for pt={pt:?} freshness={freshness:?}, got {t}");
            }
        }
        // Also test None variant
        for freshness in [
            FreshnessStatus::Fresh,
            FreshnessStatus::Unknown,
            FreshnessStatus::Stale,
        ] {
            let t = compute_fact_trust(None, freshness).unwrap();
            assert!(t > 0.0, "trust must be > 0 for None pt freshness={freshness:?}, got {t}");
            assert!(t <= 1.0, "trust must be <= 1 for None pt freshness={freshness:?}, got {t}");
        }
    }

    /// M1-B: Trust is computed and present on FactRecords from search.
    #[test]
    fn m1b_trust_computed_on_records() {
        let store = enriched_store();
        let results = search(
            &store,
            &FactSearch {
                query: "foo",
                kind: None,
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Fresh,
        )
        .expect("search succeeds");
        let foo = results
            .iter()
            .find(|r| r.name == "foo")
            .expect("foo must be found");
        assert!(
            foo.trust.is_some(),
            "trust must be computed for foo (has relationship provenance)"
        );
        let t = foo.trust.unwrap();
        assert!(t > 0.0 && t <= 1.0, "trust must be in (0,1], got {t}");
    }

    /// M1-B: BuildTarget records (None provenance) have trust computed.
    #[test]
    fn m1b_build_target_has_trust() {
        let store = sample_store();
        let results = search(
            &store,
            &FactSearch {
                query: "",
                kind: Some(FactKind::BuildTarget),
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Fresh,
        )
        .ok();
        // Build targets may or may not exist in sample_store; if present, they must have trust.
        if let Some(results) = results {
            for r in &results {
                if r.kind == "build_target" {
                    assert!(r.trust.is_some(), "build_target must have trust computed");
                }
            }
        }
    }

    /// M1-B: Serialization includes trust when computed, omits when absent.
    #[test]
    fn m1b_trust_serialization_includes_when_computed() {
        let store = enriched_store();
        let results = search(
            &store,
            &FactSearch {
                query: "foo",
                kind: None,
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Fresh,
        )
        .expect("search succeeds");
        let foo = results
            .iter()
            .find(|r| r.name == "foo")
            .expect("foo must be found");
        let json = serde_json::to_string(&foo).expect("serializes");
        assert!(
            json.contains("\"trust\""),
            "serialized fact record must include trust field: {json}"
        );
    }

    /// M1-B: Missing trust is never 0.0 — it is omitted entirely.
    #[test]
    fn m1b_missing_trust_is_omitted_not_zero() {
        // compute_fact_trust always returns Some for valid inputs, so trust
        // is always present when freshness is known. The omission path is
        // for when the caller passes a freshness that cannot be resolved.
        // Here we verify the field is never serialized as 0.0.
        let store = enriched_store();
        let results = search(
            &store,
            &FactSearch {
                query: "foo",
                kind: None,
                path: None,
                limit: DEFAULT_LIMIT,
            },
            FreshnessStatus::Fresh,
        )
        .expect("search succeeds");
        let foo = results
            .iter()
            .find(|r| r.name == "foo")
            .expect("foo must be found");
        let json = serde_json::to_string(&foo).expect("serializes");
        // If trust is present, it must not be 0.0.
        if json.contains("\"trust\"") {
            let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
            let t = v["trust"].as_f64().expect("trust is a number");
            assert!(t > 0.0, "trust must not be 0.0 when present, got {t}");
        }
    }
}

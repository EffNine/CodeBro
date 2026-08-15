//! Semantic retrieval over the fact store for the `engineering_facts` MCP
//! tool (P0.2).
//!
//! Search is a deterministic, allocation-light projection over the existing
//! immutable [`FactStore`] — no second index, no external search system.
//! Queries match against fact names, paths and signatures with a simple
//! scoring model, and results are returned as compact, LLM-friendly records
//! (not raw ids) with provenance.

use serde::Serialize;

use crate::engineering_facts::{FactKind, FactRef};
use crate::fact_store::store::FactStore;

/// Default result limit.
pub const DEFAULT_LIMIT: usize = 10;
/// Hard upper bound on returned results.
pub const MAX_LIMIT: usize = 50;

/// A compact, LLM-friendly fact record.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
    /// Internal relevance score (not part of the public contract; kept
    /// serialised for transparency but agents should not rely on it).
    #[serde(skip_serializing)]
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

/// Run a deterministic semantic search over the fact store.
///
/// Returns an error when the query is empty and no kind/path filter is
/// given: an unfiltered empty query would enumerate the whole store
/// without meaning, which is ambiguous for agents.
pub fn search(store: &FactStore, params: &FactSearch<'_>) -> Result<Vec<FactRecord>, String> {
    let query = params.query.trim().to_lowercase();
    let has_filter = params.kind.is_some() || params.path.is_some();
    if query.is_empty() && !has_filter {
        return Err("query is required (or provide a kind/path filter to enumerate)".to_string());
    }
    let path_filter = params.path.map(|p| p.to_lowercase());
    let limit = params.limit.clamp(1, MAX_LIMIT);

    let mut results: Vec<FactRecord> = Vec::new();
    for fact in store.collection().iter() {
        let Some(mut record) = record_from_fact(&fact) else {
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

/// Extract a searchable record from a fact reference, or None for kinds
/// with no meaningful name surface (e.g. relationship endpoints).
fn record_from_fact(fact: &FactRef<'_>) -> Option<FactRecord> {
    match fact {
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
            Some(r)
        }
        FactRef::Module(m) => {
            let mut r = FactRecord::new("module", m.name.clone(), 0);
            r.path = m.path.clone().or_else(|| m.location.file.clone());
            r.line = m
                .location
                .line
                .or_else(|| m.location.span.as_ref().map(|sp| sp.start.line));
            r.summary = m.metadata.description.clone();
            Some(r)
        }
        FactRef::Package(p) => {
            let mut r = FactRecord::new("package", p.name.clone(), 0);
            r.summary = p.version.clone().map(|v| format!("version {v}"));
            Some(r)
        }
        FactRef::Test(t) => {
            let mut r = FactRecord::new("test", t.name.clone(), 0);
            if let Some(loc) = &t.location {
                r.path = loc.file.clone();
                r.line = loc.line;
            }
            Some(r)
        }
        FactRef::BuildTarget(b) => {
            let mut r = FactRecord::new("build_target", b.name.clone(), 0);
            r.summary = Some(format!("{} target", b.kind.as_str()));
            Some(r)
        }
        FactRef::Dependency(d) => {
            let mut r = FactRecord::new(
                "dependency",
                format!("{} -> {}", source_label(&d.source), target_label(&d.target)),
                0,
            );
            r.summary = d.version_constraint.clone();
            Some(r)
        }
        _ => None,
    }
}

fn source_label(id: &crate::engineering_facts::FactId) -> String {
    // Dependency endpoints are typed ids; render the tail of the opaque
    // value for compactness (e.g. pkg::serde::external -> "serde").
    let s = id.as_str();
    s.rsplit("::").next().unwrap_or(s).to_string()
}

fn target_label(id: &crate::engineering_facts::FactId) -> String {
    source_label(id)
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
        FactsBuilder, ModuleFact, ModuleId, SymbolFact, SymbolId, SymbolKind, WorkspaceFact,
        WorkspaceId,
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
}

//! Engineering dependency graph edge extraction.
//!
//! Extends the call/import relationship graph with three more engineering
//! surfaces, all deterministic and evidence-carrying:
//!
//! - **API routes** — HTTP route declarations extracted by curated regexes
//!   per language family and recorded as `SymbolKind::Route` symbols owned
//!   by their module.
//! - **Documentation references** — markdown/rst files become modules;
//!   backtick-quoted identifiers that resolve to exactly one global symbol
//!   produce `Documents` edges (heuristic provenance).
//! - **Configuration artifacts** — manifest/config files become package-
//!   scoped modules with `Configures` edges to their owning package.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers

use std::collections::BTreeMap;
use std::path::Path;

use codebro_fact_store::engineering_facts::{
    FactId, FactMetadata, ModuleFact, ModuleId, PackageId, RelationshipFact, RelationshipId,
    RelationshipKind, SourceLocation, SymbolFact, SymbolId, SymbolKind, Visibility, WorkspaceId,
};

/// Files treated as configuration/manifest artifacts.
pub fn is_config_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "Cargo.toml"
            | "go.mod"
            | "package.json"
            | "pyproject.toml"
            | "setup.py"
            | "setup.cfg"
            | "requirements.txt"
            | "tsconfig.json"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "Dockerfile"
            | "Makefile"
            | ".env.example"
    ) || file_name.ends_with(".ini") && file_name != "setup.cfg"
}

/// Files treated as documentation artifacts.
pub fn is_doc_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("markdown") | Some("rst")
    )
}

/// One detected HTTP route declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRoute {
    pub method: String,
    pub path: String,
    /// 1-based line of the declaration.
    pub line: u32,
}

/// Extract HTTP routes from source text using curated per-family patterns.
pub fn extract_routes(language: &str, source: &str) -> Vec<DetectedRoute> {
    let mut out = Vec::new();
    let mut push = |method: &str, path: &str, line: u32| {
        if path.starts_with('/') && path.len() > 1 {
            out.push(DetectedRoute {
                method: method.to_uppercase(),
                path: path.to_string(),
                line,
            });
        }
    };

    for (idx, raw) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw.trim();
        match language {
            "rust" => {
                // #[get("/path")] #[post("/path")] etc. (actix/rocket style)
                if let Some(rest) = line
                    .strip_prefix("#[")
                    .and_then(|s| s.split_once('('))
                {
                    let attr = rest.0.trim().to_lowercase();
                    if matches!(
                        attr.as_str(),
                        "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
                    ) {
                        if let Some(p) = extract_quoted(rest.1) {
                            push(&attr, &p, line_no);
                        }
                    }
                }
                // axum-ish: .route("/path", get(handler)) — path side only.
                if let Some(idx) = line.find(".route(") {
                    let rest = &line[idx + 7..];
                    if let Some(p) = extract_quoted(rest) {
                        push("any", &p, line_no);
                    }
                }
            }
            "javascript" | "typescript" => {
                // app.get("/path", ...) router.post('/x', ...)
                if let Some(dot) = line.find('.') {
                    let rest = &line[dot + 1..];
                    if let Some(paren) = rest.find('(') {
                        let method = rest[..paren].trim().to_lowercase();
                        if matches!(
                            method.as_str(),
                            "get" | "post" | "put" | "delete" | "patch" | "all" | "use"
                        ) {
                            if let Some(p) = extract_quoted(&rest[paren + 1..]) {
                                push(&method, &p, line_no);
                            }
                        }
                    }
                }
            }
            "python" => {
                // @app.get("/path") @router.post("/x") @api.route("/y")
                if let Some(rest) = line.strip_prefix('@') {
                    let lower = rest.to_lowercase();
                    for m in ["get", "post", "put", "delete", "patch", "route"] {
                        if let Some(pos) = lower.find(m) {
                            if let Some(open) = lower[pos..].find('(') {
                                let after = &rest[pos + open + 1..];
                                if let Some(p) = extract_quoted(after) {
                                    push(if m == "route" { "any" } else { m }, &p, line_no);
                                }
                                break;
                            }
                        }
                    }
                }
            }
            "go" => {
                // r.HandleFunc("/path", ...) http.Handle("/x", ...)
                for pat in [".HandleFunc(", ".Handle("] {
                    if let Some(idx) = line.find(pat) {
                        let rest = &line[idx + pat.len()..];
                        if let Some(p) = extract_quoted(rest) {
                            push("any", &p, line_no);
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.path.cmp(&b.path)));
    out.dedup();
    out
}

/// Extract the first double- or single-quoted string from text.
fn extract_quoted(text: &str) -> Option<String> {
    let bytes: Vec<char> = text.chars().collect();
    let quote = bytes.first()?;
    if *quote != '"' && *quote != '\'' {
        return None;
    }
    let mut out = String::new();
    for c in bytes.iter().skip(1) {
        if *c == *quote {
            return Some(out);
        }
        out.push(*c);
    }
    None
}

/// Build route symbol facts for one parsed module.
///
/// Route symbols use deterministic ids (`sym::<rel>::route <METHOD path>`)
/// so re-indexing is stable.
pub fn route_symbols(
    ws_id: &WorkspaceId,
    rel: &str,
    language: &str,
    source: &str,
) -> Vec<SymbolFact> {
    let mid = ModuleId::new(format!("mod::{rel}"));
    extract_routes(language, source)
        .into_iter()
        .map(|r| {
            let name = format!("{} {}", r.method, r.path);
            let id = SymbolId::new(format!("sym::{}::{}", rel, name));
            let mut sf = SymbolFact::new(id, name.clone(), SymbolKind::Route);
            sf.module = Some(mid.clone());
            sf.visibility = Visibility::Public;
            sf.signature = Some(format!("{} {}", r.method, r.path));
            sf.location = SourceLocation::new()
                .with_workspace(ws_id.clone())
                .with_file(rel.to_string())
                .with_point(r.line, 0);
            sf.metadata = FactMetadata::builder()
                .language(language)
                .description(format!("HTTP route {} {}", r.method, r.path))
                .build();
            sf
        })
        .collect()
}

/// Scan documentation files and emit `Documents` relationships to uniquely
/// matched backticked identifiers.
///
/// `symbols_by_name` maps exact symbol name -> canonical fact id; names that
/// match multiple facts are ambiguous and skipped (never guessed).
/// Returns at most `max_edges_per_doc` edges per document.
pub fn doc_reference_edges(
    docs: &[(String, String)],
    symbols_by_name: &BTreeMap<String, Vec<String>>,
    max_edges_per_doc: usize,
) -> Vec<RelationshipFact> {
    let mut out = Vec::new();
    for (doc_rel, content) in docs {
        let doc_mid = ModuleId::new(format!("mod::{doc_rel}"));
        let mut emitted = 0usize;
        for ident in backtick_idents(content) {
            if emitted >= max_edges_per_doc {
                break;
            }
            if let Some(ids) = symbols_by_name.get(&ident) {
                if ids.len() == 1 {
                    let mut rel = RelationshipFact::new(
                        RelationshipId::new(format!("rel::doc::{}::{}", doc_rel, ident)),
                        RelationshipKind::Documents,
                        FactId::Module(doc_mid.clone()),
                        FactId::Symbol(codebro_fact_store::engineering_facts::SymbolId::new(
                            ids[0].clone(),
                        )),
                    );
                    rel.metadata = FactMetadata::builder()
                        .description(format!("documentation references `{}`", ident))
                        .build();
                    out.push(rel);
                    emitted += 1;
                }
            }
        }
    }
    out
}

/// Extract distinct backtick-quoted identifiers that look like code names
/// (contain `_`, `/`, `::`, or are multi-char alnum), in document order.
fn backtick_idents(content: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        match after.find('`') {
            Some(end) => {
                let ident = &after[..end];
                if looks_like_ident(ident) && seen.insert(ident.to_string()) {
                    ordered.push(ident.to_string());
                }
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    ordered
}

fn looks_like_ident(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 || s.contains(' ') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '/' | '-' | '.' | '<' | '>'))
}

/// Build a `Configures` relationship from a config artifact module to its
/// owning package.
pub fn config_module_and_edge(
    ws_id: &WorkspaceId,
    rel: &str,
    pkg: &PackageId,
) -> (ModuleFact, RelationshipFact) {
    let mid = ModuleId::new(format!("mod::{rel}"));
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let mut mf = ModuleFact::new(mid.clone(), name.clone());
    mf.package = Some(pkg.clone());
    mf.path = Some(rel.to_string());
    mf.visibility = Visibility::Public;
    mf.location = SourceLocation::new()
        .with_workspace(ws_id.clone())
        .with_file(rel.to_string());

    let mut edge = RelationshipFact::new(
        RelationshipId::new(format!("rel::configures::{}", rel)),
        RelationshipKind::Configures,
        FactId::Module(mid),
        FactId::Package(pkg.clone()),
    );
    edge.metadata = FactMetadata::builder()
        .description(format!("{} configures its package", name))
        .build();
    (mf, edge)
}

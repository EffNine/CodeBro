//! `codebro init` — engineering fact population pipeline.
//!
//! Scans a workspace, parses source files with tree-sitter, and freezes the
//! results into the canonical [`FactsModel`], persisted to
//! `.codebro/facts.json`. The MCP server (`codebro serve`) reads this file;
//! without it, `engineering_facts` returns an empty store.
//!
//! Scope (MVP): workspace, packages, modules, symbols, tests and build
//! targets. Dependencies, relationships, references, diagnostics and
//! architecture rules are follow-up milestones.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::engineering_facts::{
    BuildTargetFact, BuildTargetId, BuildTargetKind, FactsBuilder, ModuleFact, ModuleId,
    PackageFact, PackageId, SymbolFact, SymbolId, SymbolKind, TestFact, TestId, Visibility,
    WorkspaceFact, WorkspaceId,
};
use crate::engineering_facts::{
    FactsModel,
    location::{Position, SourceLocation, Span},
    metadata::FactMetadataBuilder,
};

/// Run the population pipeline for a workspace root and persist the model.
pub fn run(workspace_root: &Path) -> Result<()> {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let ws_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());

    tracing::info!("codebro init: scanning {}", root.display());

    let mut builder = FactsBuilder::new();
    let ws_id = WorkspaceId::new(format!("ws::{ws_name}"));

    // ── Workspace ────────────────────────────────────────────────────
    let mut ws = WorkspaceFact::new(ws_id.clone(), ws_name.clone());
    ws.root = Some(root.display().to_string());

    // ── Packages & build targets ─────────────────────────────────────
    let (packages, build_targets) = discover_packages(&root, &ws_id);

    // ── Source files ─────────────────────────────────────────────────
    let files = discover_source_files(&root);

    // ── Modules & symbols ────────────────────────────────────────────
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();
        let Some(language) = crate::intelligence::parser::languages::language_from_extension(
            file.extension().and_then(|e| e.to_str()).unwrap_or(""),
        ) else {
            continue;
        };

        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue, // binary/unreadable: skip
        };

        // Owning package: first package that path-prefixes this file.
        let owner_pkg_id: Option<PackageId> =
            package_for_path(&root, file, &packages).map(|p| p.id.clone());

        // Module fact per file.
        let module_name = rel.replace('/', "::").replace(".rs", "");
        let mid = ModuleId::new(format!("mod::{}", rel));
        let mut mf = ModuleFact::new(mid.clone(), module_name);
        mf.package = owner_pkg_id.clone();
        mf.path = Some(rel.clone());
        mf.visibility = Visibility::Public;
        mf.location = SourceLocation::new()
            .with_workspace(ws_id.clone())
            .with_file(rel.clone());
        builder.add_module(mf);

        // Parse symbols with the existing tree-sitter parser.
        let parsed = crate::intelligence::parser::tree_sitter::parse_file(language, file, &source)
            .with_context(|| format!("parse {} ({language})", file.display()))?;

        for sym in parsed.symbols {
            let kind = map_symbol_kind(&sym.kind);
            let mut sf = SymbolFact::new(
                SymbolId::new(format!("sym::{}::{}", rel, sym.name)),
                sym.name.clone(),
                kind,
            );
            sf.module = Some(mid.clone());
            sf.visibility = map_visibility(sym.visibility.as_deref());
            sf.signature = sym.signature.clone();
            sf.location = SourceLocation::new()
                .with_workspace(ws_id.clone())
                .with_file(rel.clone())
                .with_span(Span::new(
                    Position::new(sym.line_start, sym.column_start),
                    Position::new(sym.line_end, sym.column_end),
                ));
            if let Some(doc) = sym.doc_comment.as_deref() {
                sf.metadata = FactMetadataBuilder::new()
                    .description(doc)
                    .language(language)
                    .build();
            }
            builder.add_symbol(sf);

            // Test detection (heuristic MVP): function/method names that
            // look like tests, or files whose path mentions "test".
            let looks_like_test_file = rel.contains("test");
            let looks_like_test_fn = (sym.name.starts_with("test_") || sym.name.ends_with("_test"))
                && matches!(sym.kind, crate::intelligence::parser::SymbolKind::Function);
            if looks_like_test_file || looks_like_test_fn {
                let mut tf = TestFact::new(
                    TestId::new(format!("test::{}::{}", rel, sym.name)),
                    sym.name.clone(),
                );
                tf.target = Some(crate::engineering_facts::FactId::Symbol(SymbolId::new(
                    format!("sym::{}::{}", rel, sym.name),
                )));
                tf.location = Some(
                    SourceLocation::new()
                        .with_workspace(ws_id.clone())
                        .with_file(rel.clone())
                        .with_span(Span::new(
                            Position::new(sym.line_start, sym.column_start),
                            Position::new(sym.line_end, sym.column_end),
                        )),
                );
                builder.add_test(tf);
            }
        }
    }

    // ── Assemble package/workspace references ────────────────────────
    for pkg in &packages {
        let mut pf = PackageFact::new(pkg.id.clone(), pkg.name.clone());
        pf.workspace = Some(ws_id.clone());
        pf.language = Some(pkg.language.clone());
        pf.version = pkg.version.clone();
        let targets: Vec<BuildTargetId> = build_targets
            .iter()
            .filter(|b| b.package == Some(pkg.id.clone()))
            .map(|b| b.id.clone())
            .collect();
        pf.build_targets = targets;
        builder.add_package(pf);
    }
    for bt in &build_targets {
        builder.add_build_target(bt.clone());
    }
    ws.packages = packages.iter().map(|p| p.id.clone()).collect();
    builder.add_workspace(ws);

    let model: FactsModel = builder.build();

    // ── Persist ──────────────────────────────────────────────────────
    let codebro_dir = root.join(".codebro");
    std::fs::create_dir_all(&codebro_dir).context("create .codebro directory")?;
    let out = codebro_dir.join("facts.json");
    let json = serde_json::to_string_pretty(&model).context("serialize facts model")?;
    std::fs::write(&out, json).context("write .codebro/facts.json")?;

    let counts = model.counts();
    println!("codebro init complete");
    println!("  workspace:   {ws_name}");
    println!("  packages:    {}", counts.packages);
    println!("  modules:     {}", counts.modules);
    println!("  symbols:     {}", counts.symbols);
    println!("  tests:       {}", counts.tests);
    println!("  build targets: {}", counts.build_targets);
    println!("  facts file:  {}", out.display());

    Ok(())
}

/// A lightweight package descriptor produced by manifest discovery.
struct DiscoveredPackage {
    id: PackageId,
    name: String,
    version: Option<String>,
    language: String,
    path: PathBuf,
}

/// Read `Cargo.toml` for the workspace root; fall back to a single
/// root-level package when no manifest is found.
fn discover_packages(
    root: &Path,
    ws_id: &WorkspaceId,
) -> (Vec<DiscoveredPackage>, Vec<BuildTargetFact>) {
    let cargo = root.join("Cargo.toml");
    if cargo.exists() {
        if let Some((pkg, targets)) = parse_cargo_package(root, ws_id) {
            return (vec![pkg], targets);
        }
    }

    // Fallback: a single unnamed root package.
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "root".to_string());
    let id = PackageId::new(format!("pkg::{name}"));
    let mut fallback_target = BuildTargetFact::new(
        BuildTargetId::new(format!("build::{name}")),
        name.clone(),
        BuildTargetKind::Binary,
    );
    fallback_target.language = Some("unknown".to_string());

    (
        vec![DiscoveredPackage {
            id,
            name,
            version: None,
            language: "unknown".to_string(),
            path: root.to_path_buf(),
        }],
        vec![fallback_target],
    )
}

/// Parse a Cargo manifest into a package plus bin/lib targets.
fn parse_cargo_package(
    root: &Path,
    ws_id: &WorkspaceId,
) -> Option<(DiscoveredPackage, Vec<BuildTargetFact>)> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let value: toml::Value = text.parse().ok()?;

    let package = value.get("package")?;
    let name = package.get("name")?.as_str()?.to_string();
    let version = package
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let id = PackageId::new(format!("pkg::{name}"));

    let mut targets = Vec::new();

    // Library target: [lib] or implicit src/lib.rs.
    if root.join("src/lib.rs").exists() {
        let lib_name = value
            .get("lib")
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or(&name)
            .to_string();
        let mut t = BuildTargetFact::new(
            BuildTargetId::new(format!("build::{lib_name}")),
            lib_name,
            BuildTargetKind::Library,
        );
        t.package = Some(id.clone());
        t.language = Some("rust".to_string());
        targets.push(t);
    }

    // Binary targets: [[bin]] or implicit src/main.rs.
    if root.join("src/main.rs").exists() {
        let mut t = BuildTargetFact::new(
            BuildTargetId::new(format!("build::{name}")),
            name.clone(),
            BuildTargetKind::Binary,
        );
        t.package = Some(id.clone());
        t.language = Some("rust".to_string());
        targets.push(t);
    }
    if let Some(bins) = value.get("bin").and_then(|b| b.as_array()) {
        for bin in bins {
            let bin_name = bin
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(&name)
                .to_string();
            let mut t = BuildTargetFact::new(
                BuildTargetId::new(format!("build::{bin_name}")),
                bin_name,
                BuildTargetKind::Binary,
            );
            t.package = Some(id.clone());
            t.language = Some("rust".to_string());
            targets.push(t);
        }
    }

    // Test targets: [test] entries.
    if let Some(tests) = value.get("test").and_then(|t| t.as_array()) {
        for t in tests {
            let test_name = t
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("test")
                .to_string();
            let mut bt = BuildTargetFact::new(
                BuildTargetId::new(format!("build::{test_name}")),
                test_name,
                BuildTargetKind::Test,
            );
            bt.package = Some(id.clone());
            bt.language = Some("rust".to_string());
            targets.push(bt);
        }
    }

    if targets.is_empty() {
        // Unknown target shape: still register the package with a generic target.
        let mut t = BuildTargetFact::new(
            BuildTargetId::new(format!("build::{name}")),
            name.clone(),
            BuildTargetKind::Unknown,
        );
        t.package = Some(id.clone());
        t.language = Some("rust".to_string());
        targets.push(t);
    }

    Some((
        DiscoveredPackage {
            id,
            name,
            version,
            language: "rust".to_string(),
            path: root.to_path_buf(),
        },
        targets,
    ))
}

/// Find the first package whose root path prefixes `file`.
fn package_for_path<'a>(
    root: &Path,
    file: &Path,
    packages: &'a [DiscoveredPackage],
) -> Option<&'a DiscoveredPackage> {
    let rel = file.strip_prefix(root).ok()?;
    for pkg in packages {
        let pkg_rel = pkg.path.strip_prefix(root).unwrap_or(&pkg.path);
        if pkg_rel.as_os_str().is_empty() || rel.starts_with(pkg_rel) {
            return Some(pkg);
        }
    }
    packages.first()
}

/// Discover source files, skipping common build/vendor directories.
fn discover_source_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            !matches!(
                name.as_str(),
                ".git" | ".codebro" | "target" | "node_modules" | "dist" | "build" | "vendor"
            )
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("");
        if crate::intelligence::parser::languages::language_from_extension(ext).is_some() {
            out.push(entry.path().to_path_buf());
        }
    }
    out.sort();
    out
}

/// Map the parser's symbol kind onto the canonical facts model kind.
fn map_symbol_kind(kind: &crate::intelligence::parser::SymbolKind) -> SymbolKind {
    use crate::intelligence::parser::SymbolKind as P;
    match kind {
        P::Function => SymbolKind::Function,
        P::Method => SymbolKind::Method,
        P::Class => SymbolKind::Class,
        P::Struct => SymbolKind::Struct,
        P::Enum => SymbolKind::Enum,
        P::Trait => SymbolKind::Trait,
        P::Interface => SymbolKind::Interface,
        P::TypeAlias => SymbolKind::TypeAlias,
        P::Variable => SymbolKind::Variable,
        P::Constant => SymbolKind::Constant,
        P::Field => SymbolKind::Field,
        P::Parameter => SymbolKind::Parameter,
        P::Macro => SymbolKind::Macro,
        P::Constructor => SymbolKind::Constructor,
        P::Module => SymbolKind::Namespace,
        P::Import | P::Export => SymbolKind::Import,
        P::Impl => SymbolKind::Unknown,
    }
}

/// Map a parser visibility string onto the canonical model.
fn map_visibility(vis: Option<&str>) -> Visibility {
    match vis {
        Some("pub") => Visibility::Public,
        Some("pub(crate)") | Some("pub(super)") | Some("pub(in") => Visibility::Internal,
        Some("protected") => Visibility::Protected,
        Some("private") => Visibility::Private,
        _ => Visibility::Unknown,
    }
}

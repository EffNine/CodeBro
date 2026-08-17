//! `codebro init` — engineering fact population pipeline.
//!
//! Scans a workspace, parses source files with tree-sitter, and freezes the
//! results into the canonical [`FactsModel`], persisted to
//! `.codebro/facts.json`. The MCP server (`codebro serve`) reads this file;
//! without it, `engineering_facts` returns an empty store.
//!
//! Scope: workspace, packages, modules, symbols, tests, build targets,
//! package dependencies (from Cargo.toml), and cross-module relationship
//! facts inferred from symbol name co-occurrence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::engineering_facts::{
    location::{Position, SourceLocation, Span},
    metadata::FactMetadataBuilder,
    FactsModel,
};
use crate::engineering_facts::{
    BuildTargetFact, BuildTargetId, BuildTargetKind, DependencyFact, DependencyId, DependencyKind,
    FactId, FactsBuilder, ModuleFact, ModuleId, PackageFact, PackageId, SymbolFact, SymbolId,
    SymbolKind, TestFact, TestId, Visibility, WorkspaceFact, WorkspaceId,
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
    let mut collected_modules: Vec<ModuleFact> = Vec::new();
    let mut collected_symbols: Vec<SymbolFact> = Vec::new();
    // Collect AST-derived calls and imports per file for relationship building.
    let mut all_calls: Vec<crate::intelligence::parser::ParseCall> = Vec::new();
    let mut all_imports: Vec<crate::intelligence::parser::ParseImport> = Vec::new();
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
        builder.add_module(mf.clone());
        collected_modules.push(mf);

        // Parse symbols with the existing tree-sitter parser.
        let parsed = crate::intelligence::parser::tree_sitter::parse_file(language, file, &source)
            .with_context(|| format!("parse {} ({language})", file.display()))?;

        for sym in parsed.symbols {
            let kind = map_symbol_kind(&sym.kind);
            // Uniqueness: same name can appear multiple times per file
            // (e.g. method `new` on many structs, or a struct and its impl
            // block). Disambiguate with kind + line so every symbol gets a
            // stable, unique id without changing the source-level identity.
            let sym_id = format!(
                "sym::{}::{}_{}@{}",
                rel,
                sym.name,
                kind.as_str(),
                sym.line_start
            );
            let mut sf = SymbolFact::new(SymbolId::new(sym_id), sym.name.clone(), kind);
            sf.module = Some(mid.clone());
            sf.visibility = map_visibility(sym.visibility.as_deref());
            sf.signature = sym.signature.clone();
            sf.location = SourceLocation::new()
                .with_workspace(ws_id.clone())
                .with_file(rel.clone())
                .with_point(sym.line_start, sym.column_start)
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
            builder.add_symbol(sf.clone());
            collected_symbols.push(sf);

            // Collect AST calls and imports from this file.
            all_calls.extend(parsed.calls.iter().cloned());
            all_imports.extend(parsed.import_targets.iter().cloned());

            // Test detection (heuristic MVP): function/method names that
            // look like tests, or files whose path mentions "test".
            let looks_like_test_file = rel.contains("test");
            let looks_like_test_fn = (sym.name.starts_with("test_") || sym.name.ends_with("_test"))
                && matches!(sym.kind, crate::intelligence::parser::SymbolKind::Function);
            if looks_like_test_file || looks_like_test_fn {
                let mut tf = TestFact::new(
                    TestId::new(format!(
                        "test::{}::{}_{}@{}",
                        rel,
                        sym.name,
                        kind.as_str(),
                        sym.line_start
                    )),
                    sym.name.clone(),
                );
                tf.target = Some(crate::engineering_facts::FactId::Symbol(SymbolId::new(
                    format!(
                        "sym::{}::{}_{}@{}",
                        rel,
                        sym.name,
                        kind.as_str(),
                        sym.line_start
                    ),
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
    let mut external_crates: Vec<DiscoveredPackage> = Vec::new();
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

        // Dependency links: source = this package, target = an external
        // crate package fact (created below so endpoints resolve).
        for dep in &pkg.dependencies {
            let target_id = PackageId::new(format!("pkg::{crate}::external", crate = dep.name));
            let dep_id = DependencyId::new(format!("dep::{}->{}", pkg.name, dep.name));
            let mut df = DependencyFact::new(
                dep_id,
                FactId::Package(pkg.id.clone()),
                FactId::Package(target_id.clone()),
            );
            df.kind = dep.kind;
            df.version_constraint = dep.version.clone();
            builder.add_dependency(df);

            external_crates.push(DiscoveredPackage {
                id: target_id,
                name: dep.name.clone(),
                version: dep.version.clone(),
                language: "unknown".to_string(),
                path: root.join("."),
                dependencies: Vec::new(),
            });
        }
    }
    // External crate stubs: package facts so dependency endpoints resolve
    // and the graph is queryable. Workspace = None (they are not part of
    // this project).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ext in &external_crates {
        if !seen.insert(ext.id.as_str().to_string()) {
            continue;
        }
        let mut ef = PackageFact::new(ext.id.clone(), ext.name.clone());
        ef.language = Some("unknown".to_string());
        ef.version = ext.version.clone();
        builder.add_package(ef);
    }

    for bt in &build_targets {
        builder.add_build_target(bt.clone());
    }
    ws.packages = packages.iter().map(|p| p.id.clone()).collect();
    builder.add_workspace(ws);

    // ── Build cross-module relationship facts ────────────────────────
    let rel_count = crate::impact::relationships::build_relationships(
        &mut builder,
        &collected_modules,
        &collected_symbols,
        &all_calls,
        &all_imports,
    );
    if rel_count > 0 {
        println!("  relationships: {rel_count}");
    }

    // Drop intermediate collected data early — the builder now owns
    // all the facts. Keeping these vectors alive during serialization
    // would duplicate the symbol/call/import data in RAM.
    drop(collected_modules);
    drop(collected_symbols);
    drop(all_calls);
    drop(all_imports);
    drop(files);

    // Capture generation-time repository state for freshness comparison.
    let gen_state = crate::sandbox::RepoState::capture(&root);

    let model: FactsModel = builder.build();
    let model = if let Some(state) = gen_state {
        model.with_generation_repo_state(state)
    } else {
        model
    };

    // ── Persist ──────────────────────────────────────────────────────
    let codebro_dir = root.join(".codebro");
    std::fs::create_dir_all(&codebro_dir).context("create .codebro directory")?;
    let out = codebro_dir.join("facts.json");
    let file = std::fs::File::create(&out).context("create facts.json")?;
    serde_json::to_writer_pretty(file, &model).context("serialize facts model")?;

    let counts = model.counts();
    println!("codebro init complete");
    println!("  workspace:   {ws_name}");
    println!("  packages:    {}", counts.packages);
    println!("  modules:     {}", counts.modules);
    println!("  symbols:     {}", counts.symbols);
    println!("  tests:       {}", counts.tests);
    println!("  build targets: {}", counts.build_targets);
    println!("  dependencies: {}", counts.dependencies);
    println!("  relationships: {}", counts.relationships);
    println!("  references:    {}", counts.references);
    println!("  facts file:  {}", out.display());

    Ok(())
}

/// A single declared dependency (crate name + version constraint + kind).
struct DiscoveredDependency {
    name: String,
    version: Option<String>,
    kind: DependencyKind,
}

/// A lightweight package descriptor produced by manifest discovery.
struct DiscoveredPackage {
    id: PackageId,
    name: String,
    version: Option<String>,
    language: String,
    path: PathBuf,
    dependencies: Vec<DiscoveredDependency>,
}

/// Read `Cargo.toml` (Rust) or `go.mod` (Go) for the workspace root; fall
/// back to a single root-level package when no manifest is found.
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
    let go_mod = root.join("go.mod");
    if go_mod.exists() {
        if let Some((pkg, targets)) = parse_go_package(root, ws_id) {
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
        BuildTargetId::new(format!("build::bin::{name}")),
        name.clone(),
        BuildTargetKind::Binary,
    );
    fallback_target.language = Some("unknown".to_string());
    fallback_target.package = Some(id.clone());

    (
        vec![DiscoveredPackage {
            id,
            name,
            version: None,
            language: "unknown".to_string(),
            path: root.to_path_buf(),
            dependencies: Vec::new(),
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
            BuildTargetId::new(format!("build::lib::{lib_name}")),
            lib_name,
            BuildTargetKind::Library,
        );
        t.package = Some(id.clone());
        t.language = Some("rust".to_string());
        targets.push(t);
    }

    // Binary targets: [[bin]] entries, or the implicit src/main.rs binary
    // only when no explicit [[bin]] section exists (Cargo infers main.rs as
    // a binary named after the package when [[bin]] is absent; when it is
    // present, the explicit entries are authoritative).
    let explicit_bins = value.get("bin").and_then(|b| b.as_array());
    if root.join("src/main.rs").exists() && explicit_bins.is_none() {
        let mut t = BuildTargetFact::new(
            BuildTargetId::new(format!("build::bin::{name}")),
            name.clone(),
            BuildTargetKind::Binary,
        );
        t.package = Some(id.clone());
        t.language = Some("rust".to_string());
        targets.push(t);
    }
    if let Some(bins) = explicit_bins {
        for bin in bins {
            let bin_name = bin
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(&name)
                .to_string();
            let mut t = BuildTargetFact::new(
                BuildTargetId::new(format!("build::bin::{bin_name}")),
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
                BuildTargetId::new(format!("build::test::{test_name}")),
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
            BuildTargetId::new(format!("build::bin::{name}")),
            name.clone(),
            BuildTargetKind::Unknown,
        );
        t.package = Some(id.clone());
        t.language = Some("rust".to_string());
        targets.push(t);
    }

    // ── Dependencies ──────────────────────────────────────────────────
    // [dependencies] + [dev-dependencies] + [build-dependencies].
    let mut dependencies: Vec<DiscoveredDependency> = Vec::new();
    for (section, kind) in [
        ("dependencies", DependencyKind::Direct),
        ("dev-dependencies", DependencyKind::Dev),
        ("build-dependencies", DependencyKind::Build),
    ] {
        let Some(table) = value.get(section).and_then(|v| v.as_table()) else {
            continue;
        };
        for (dep_name, dep_value) in table {
            // Simple form: `serde = "1"` or `serde = { version = "1", optional = true }`.
            let (version, optional) = match dep_value {
                toml::Value::String(v) => (Some(v.clone()), false),
                toml::Value::Table(t) => (
                    t.get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    t.get("optional").and_then(|v| v.as_bool()).unwrap_or(false),
                ),
                _ => (None, false),
            };
            let effective_kind = if optional {
                DependencyKind::Optional
            } else {
                kind
            };
            dependencies.push(DiscoveredDependency {
                name: dep_name.clone(),
                version,
                kind: effective_kind,
            });
        }
    }

    Some((
        DiscoveredPackage {
            id,
            name,
            version,
            language: "rust".to_string(),
            path: root.to_path_buf(),
            dependencies,
        },
        targets,
    ))
}

/// Parse a Go `go.mod` into a package plus a single binary target and its
/// dependencies. Direct and `// indirect` requires are distinguished.
fn parse_go_package(
    root: &Path,
    _ws_id: &WorkspaceId,
) -> Option<(DiscoveredPackage, Vec<BuildTargetFact>)> {
    let text = std::fs::read_to_string(root.join("go.mod")).ok()?;
    let module = text
        .lines()
        .find(|l| l.trim_start().starts_with("module "))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())?;
    let name = module.rsplit('/').next().unwrap_or(&module).to_string();
    let id = PackageId::new(format!("pkg::{name}"));

    let mut target = BuildTargetFact::new(
        BuildTargetId::new(format!("build::bin::{name}")),
        name.clone(),
        BuildTargetKind::Binary,
    );
    target.package = Some(id.clone());
    target.language = Some("go".to_string());

    // Dependencies: lines inside `require (` blocks, `name version [// indirect]`.
    let mut dependencies: Vec<DiscoveredDependency> = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "require (" {
            in_require = true;
            continue;
        }
        if in_require && trimmed == ")" {
            in_require = false;
            continue;
        }
        if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
            let mut parts = trimmed.split_whitespace();
            if let (Some(dep), Some(_ver)) = (parts.next(), parts.next()) {
                let indirect = trimmed.contains("// indirect");
                dependencies.push(DiscoveredDependency {
                    name: dep.to_string(),
                    version: None,
                    kind: if indirect {
                        DependencyKind::Transitive
                    } else {
                        DependencyKind::Direct
                    },
                });
            }
        }
    }

    Some((
        DiscoveredPackage {
            id,
            name,
            version: None,
            language: "go".to_string(),
            path: root.to_path_buf(),
            dependencies,
        },
        vec![target],
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
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_empty_dir_produces_valid_model() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path()).unwrap();
        let facts = dir.path().join(".codebro/facts.json");
        assert!(facts.exists());
        let model: FactsModel =
            serde_json::from_str(&std::fs::read_to_string(facts).unwrap()).unwrap();
        // Fallback package + build target, no modules/symbols.
        assert_eq!(model.workspaces().len(), 1);
        assert_eq!(model.packages().len(), 1);
        assert_eq!(model.symbols().len(), 0);
    }

    #[test]
    fn init_scans_rust_files_into_symbols() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "pub struct Config { pub name: String }\npub fn main() {}\n#[test]\nfn test_x() {}\n",
        )
        .unwrap();
        let r = run(dir.path());
        assert!(r.is_ok(), "run failed: {r:?}");
        let facts = dir.path().join(".codebro/facts.json");
        assert!(facts.exists(), "facts.json missing: {:?}", dir.path());
        let model: FactsModel =
            serde_json::from_str(&std::fs::read_to_string(facts).unwrap()).unwrap();
        assert!(
            model.symbols().len() >= 2,
            "expected symbols, got {}",
            model.symbols().len()
        );
        let names: Vec<&str> = model.symbols().iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Config"));
        assert!(names.contains(&"main"));
        // Test detection heuristic picks test_x.
        assert!(model.tests().len() >= 1);
    }

    #[test]
    fn init_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "pub fn a() {}\npub fn b() {}\n",
        )
        .unwrap();
        run(dir.path()).unwrap();
        let first = std::fs::read(dir.path().join(".codebro/facts.json")).unwrap();
        run(dir.path()).unwrap();
        let second = std::fs::read(dir.path().join(".codebro/facts.json")).unwrap();
        assert_eq!(first, second, "re-init must be byte-identical");
    }

    #[test]
    fn generation_repo_state_captured_before_fact_generation() {
        // Regression test: generation_repo_state must represent the repo
        // state at the time facts are generated, not after serialization.
        // The invariant is: capture R0 -> generate facts from R0 -> store R0 -> serialize.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"timing-test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "pub fn hello() -> i32 { 42 }\n",
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
            .args(["commit", "-m", "initial"])
            .output()
            .expect("git commit succeeded");

        // Capture the repo state BEFORE running init.
        let pre_capture = crate::sandbox::RepoState::capture(&dir.path().to_path_buf());
        assert!(
            pre_capture.is_some(),
            "capture must succeed in a git repo"
        );
        let pre_state = pre_capture.unwrap();

        run(dir.path()).unwrap();

        // Load the model from the serialized facts.json.
        let facts = dir.path().join(".codebro/facts.json");
        let model: FactsModel =
            serde_json::from_str(&std::fs::read_to_string(facts).unwrap()).unwrap();

        // The generation_repo_state must be Some and match the pre-generation capture.
        let gen_state = model.generation_repo_state().expect("generation_repo_state must be set");
        assert_eq!(
            gen_state.working_tree_hash, pre_state.working_tree_hash,
            "generation_repo_state must reflect pre-generation repo state"
        );
        assert_eq!(
            gen_state.commit_sha, pre_state.commit_sha,
            "generation_repo_state must reflect pre-generation commit SHA"
        );
    }
}

#[cfg(test)]
mod go_tests {
    use super::*;

    #[test]
    fn parse_go_mod_extracts_direct_and_indirect() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/example/myapp\n\ngo 1.21\n\nrequire (\n\tgithub.com/fiber v1.0.0\n\tgithub.com/uuid v1.6.0 // indirect\n)\n",
        )
        .unwrap();
        let (pkg, targets) = discover_packages(dir.path(), &WorkspaceId::new("ws::x"));
        assert_eq!(pkg.len(), 1);
        assert_eq!(pkg[0].name, "myapp");
        assert_eq!(pkg[0].language, "go");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].language.as_deref(), Some("go"));

        let deps = &pkg[0].dependencies;
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "github.com/fiber");
        assert_eq!(deps[0].kind, DependencyKind::Direct);
        assert_eq!(deps[1].name, "github.com/uuid");
        assert_eq!(deps[1].kind, DependencyKind::Transitive);
    }

    #[test]
    fn go_mod_roundtrip_produces_valid_store() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/example/app\n\ngo 1.21\n\nrequire (\n\tgithub.com/x v1.0.0\n)\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("internal/breaker")).unwrap();
        std::fs::write(
            dir.path().join("internal/breaker/breaker.go"),
            "package breaker\ntype Breaker struct{}\nfunc (b *Breaker) Allow() bool { return true }\n",
        )
        .unwrap();
        let r = run(dir.path());
        assert!(r.is_ok(), "run failed: {r:?}");
        let facts = dir.path().join(".codebro/facts.json");
        assert!(facts.exists(), "facts.json missing: {:?}", dir.path());
        let model: FactsModel =
            serde_json::from_str(&std::fs::read_to_string(facts).unwrap()).unwrap();
        assert_eq!(model.dependencies().len(), 1);
        assert_eq!(model.symbols().len(), 2); // Breaker + Allow method
        let names: Vec<&str> = model.symbols().iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Allow"));
    }
}

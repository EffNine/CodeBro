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
    let mut impl_scopes: Vec<crate::impact::relationships::ImplScope> = Vec::new();
    let mut skipped_oversized: usize = 0;
    // Machine-generated "source" files that embed datasets (common in ML
    // repos: data.py with megabytes of array literals, bundled minified
    // bundles, snapshot dumps with a .js/.py extension) explode parse
    // memory and CPU for zero engineering value. Real hand-written source
    // stays far below this bound.
    const MAX_SOURCE_FILE_BYTES: u64 = 512 * 1024;
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

        // Size gate before any read: one stat instead of loading the
        // whole file into memory just to reject it.
        match std::fs::metadata(file) {
            Ok(meta) if meta.len() <= MAX_SOURCE_FILE_BYTES => {}
            _ => {
                skipped_oversized += 1;
                continue;
            }
        }

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
        let mut parsed =
            crate::intelligence::parser::tree_sitter::parse_file(language, file, &source)
                .with_context(|| format!("parse {} ({language})", file.display()))?;

        // The parser only knows the bare file name; rewrite call/import
        // locations to the workspace-relative path so caller resolution and
        // import resolution match symbol/module facts (which are keyed by
        // the relative path).
        for call in &mut parsed.calls {
            call.caller_file = rel.clone();
        }
        for imp in &mut parsed.import_targets {
            imp.file = rel.clone();
        }
        all_calls.extend(parsed.calls.iter().cloned());
        all_imports.extend(parsed.import_targets.iter().cloned());

        // Record impl block extents so method calls can be attributed to
        // their self type during relationship resolution.
        use crate::intelligence::parser::SymbolKind as P;
        for sym in &parsed.symbols {
            if matches!(sym.kind, P::Impl) && sym.name != "unknown" {
                impl_scopes.push(crate::impact::relationships::ImplScope {
                    file: rel.clone(),
                    start: sym.line_start,
                    end: sym.line_end,
                    type_name: sym.name.clone(),
                });
            }
        }

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
        &impl_scopes,
    );
    if rel_count > 0 {
        println!("  relationships: {rel_count}");
    }

    // Derive a deterministic architecture summary and the top modules by
    // symbol count from the collected data, before it is dropped.
    let arch_summary =
        architecture_summary(&collected_modules, &collected_symbols, packages.len());
    let top_modules = top_modules_by_symbols(&collected_modules, &collected_symbols, 8);

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
    let bytes = serde_json::to_vec_pretty(&model).context("serialize facts model")?;
    // Atomic + durable: staged temp file, fsync, rename. A crash mid-write
    // can never truncate an existing facts store.
    crate::persistence::write_atomic(&out, &bytes).context("persist facts.json")?;

    // ── Refresh project identity ─────────────────────────────────────
    // Fill only what no one has authored yet: re-running init never
    // clobbers curated identity data (goals, constraints, decisions).
    match refresh_identity(&root, &ws_name, &arch_summary, &top_modules) {
        Ok(true) => println!("  identity:    refreshed"),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!("identity refresh skipped: {e}");
            println!("  identity:    refresh failed ({e})");
        }
    }

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
    if skipped_oversized > 0 {
        println!(
            "  skipped:     {skipped_oversized} oversized source files (>{} KiB)",
            MAX_SOURCE_FILE_BYTES / 1024
        );
    }
    println!("  facts file:  {}", out.display());

    Ok(())
}

/// Deterministic one-line architecture summary from collected facts.
///
/// Example: "359 modules in 24 source areas; primary areas by symbol
/// count: mcp (412), engineering_facts (388), intelligence (350)."
fn architecture_summary(
    modules: &[ModuleFact],
    symbols: &[SymbolFact],
    package_count: usize,
) -> String {
    // module id → workspace-relative path.
    let path_of: std::collections::HashMap<&ModuleId, &str> = modules
        .iter()
        .filter_map(|m| m.path.as_deref().map(|p| (&m.id, p)))
        .collect();

    // Symbols per source area: the directory component after `src/` (or
    // "(root)" for top-level files).
    let mut area_symbols: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut area_modules: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for m in modules {
        let area = source_area(m.path.as_deref().unwrap_or(""));
        *area_modules.entry(area).or_default() += 1;
    }
    for s in symbols {
        let area = s
            .module
            .as_ref()
            .and_then(|mid| path_of.get(mid).copied())
            .map(source_area)
            .unwrap_or_else(|| "(root)".to_string());
        *area_symbols.entry(area).or_default() += 1;
    }

    let mut ranked: Vec<(String, usize)> = area_symbols.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    // Entry-point noise ("src/main.rs" alone) should not crowd out real
    // source areas when there are any.
    if ranked.len() > 1 {
        ranked.retain(|(area, _)| area != "(root)");
    }
    let top: Vec<String> = ranked
        .iter()
        .take(3)
        .map(|(area, n)| format!("{area} ({n})"))
        .collect();

    format!(
        "{} modules across {} packages, organized into {} source areas; primary areas by symbol count: {}",
        modules.len(),
        package_count,
        area_modules.len(),
        if top.is_empty() {
            "none".to_string()
        } else {
            top.join(", ")
        }
    )
}

/// Source area for a workspace-relative file path: the first directory
/// component after an optional leading `src/`, else "(root)".
fn source_area(path: &str) -> String {
    let stripped = path.strip_prefix("src/").unwrap_or(path);
    match stripped.split_once('/') {
        Some((area, _)) => area.to_string(),
        None => "(root)".to_string(),
    }
}

/// Up to `limit` module paths ranked by contained symbol count
/// (descending; ties broken by path for determinism).
fn top_modules_by_symbols(
    modules: &[ModuleFact],
    symbols: &[SymbolFact],
    limit: usize,
) -> Vec<String> {
    let mut counts: std::collections::HashMap<&ModuleId, usize> =
        std::collections::HashMap::new();
    for s in symbols {
        if let Some(mid) = &s.module {
            *counts.entry(mid).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = modules
        .iter()
        .filter_map(|m| {
            let path = m.path.as_deref()?;
            let n = counts.get(&m.id).copied().unwrap_or(0);
            Some((path, n))
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked.into_iter().take(limit).map(|(path, _)| path.to_string()).collect()
}

/// Refresh `.codebro/` project identity for the initialized workspace.
///
/// Deterministic surface inference fills only fields that are still empty,
/// so human- or agent-authored goals/constraints/decisions survive re-init.
/// Returns `Ok(true)` when changes were applied, `Ok(false)` when identity
/// was already complete.
fn refresh_identity(
    root: &Path,
    ws_name: &str,
    arch_summary: &str,
    top_modules: &[String],
) -> Result<bool> {
    use crate::project_identity::{IdentityChanges, ProjectIdentityUpdater};

    let inferred = crate::project_identity::infer_identity(root);

    let mut runtime = crate::project_identity::ProjectIdentityRuntime::new(root);
    let current = match runtime.load() {
        Ok(_) => runtime.snapshot(),
        Err(_) => {
            runtime.create_minimal(ws_name, primary_language(root))?;
            runtime.snapshot()
        }
    };

    let mut changes = IdentityChanges::new();
    if current.description.is_none() {
        changes.set_description = inferred.description;
    }
    if current.repository_url.is_none() {
        changes.set_repository_url = inferred.repository_url;
    }
    if current.build_system.is_none() {
        changes.set_build_system = inferred.build_system;
    }
    if current.package_manager.is_none() {
        changes.set_package_manager = inferred.package_manager;
    }
    if current.testing_framework.is_none() {
        changes.set_testing_framework = inferred.testing_framework;
    }
    for fw in inferred.frameworks {
        if !current.frameworks.contains(&fw) {
            changes.add_frameworks.push(fw);
        }
    }
    for file in inferred.important_files {
        if !current.important_files.contains(&file) {
            changes.add_important_files.push(file);
        }
    }
    if !arch_summary.is_empty() && machine_generated_summary(current.architecture_summary.as_deref())
    {
        changes.update_architecture_summary = Some(arch_summary.to_string());
    }
    for module in top_modules {
        if !current.known_modules.contains(module) {
            changes.add_modules.push(module.clone());
        }
    }

    // Mined documentation content — human-authored text parsed verbatim
    // from the workspace's own docs, never guessed.
    for mined in &inferred.decisions {
        if current
            .engineering_decisions
            .iter()
            .any(|d| d.id == mined.id)
        {
            continue;
        }
        use crate::project_identity::DecisionStatus;
        let decision = crate::project_identity::EngineeringDecision::new(
            mined.id.clone(),
            mined.title.clone(),
            mined
                .description
                .clone()
                .unwrap_or_else(|| mined.title.clone()),
            Some(mined.source_file.clone()),
        );
        let status = match mined.status {
            "accepted" => DecisionStatus::Accepted,
            "deprecated" => DecisionStatus::Deprecated,
            "superseded" => DecisionStatus::Superseded,
            _ => DecisionStatus::Proposed,
        };
        changes
            .add_decisions
            .push(decision.with_status(status));
    }
    for convention in &inferred.conventions {
        if !current.coding_conventions.contains(convention)
            && !changes.add_conventions.contains(convention)
        {
            changes.add_conventions.push(convention.clone());
        }
    }
    for milestone in &inferred.milestones {
        if !current.recent_milestones.contains(milestone)
            && !changes.add_milestones.contains(milestone)
        {
            changes.add_milestones.push(milestone.clone());
        }
    }

    if changes.is_empty() {
        return Ok(false);
    }

    let mut updater = ProjectIdentityUpdater::new(root);
    match updater.update(&current, changes) {
        Some(result) if result.applied => Ok(true),
        Some(result) => Err(anyhow::anyhow!(
            "identity update rejected by validation: {:?}",
            result.diagnostics.validation_errors
        )),
        None => Ok(false),
    }
}

/// True when the existing summary was itself generated by this pipeline
/// (matches the deterministic template), so init may refresh it. Prose
/// summaries authored by humans or agents are never overwritten.
fn machine_generated_summary(existing: Option<&str>) -> bool {
    match existing {
        None => true,
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return true;
            }
            trimmed.contains(" modules across ")
                || trimmed.contains(" modules in ")
                || trimmed.starts_with("organized into ")
        }
    }
}

/// Primary language of the workspace based on which manifest exists.
fn primary_language(root: &Path) -> &'static str {
    if root.join("Cargo.toml").is_file() {
        "rust"
    } else if root.join("go.mod").is_file() {
        "go"
    } else if root.join("tsconfig.json").is_file() || root.join("package.json").is_file() {
        "typescript"
    } else {
        "unknown"
    }
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
            // The same crate may appear in several sections (e.g.
            // [dependencies] and [dev-dependencies]). The dependency id is
            // keyed by name only, so keep one entry — the most product-
            // relevant kind wins (direct > build > dev).
            if let Some(existing) = dependencies
                .iter_mut()
                .find(|d| d.name == *dep_name)
            {
                let rank = |k: DependencyKind| match k {
                    DependencyKind::Direct => 0,
                    DependencyKind::Build => 1,
                    _ => 2,
                };
                if rank(effective_kind) < rank(existing.kind) {
                    existing.kind = effective_kind;
                    existing.version = version;
                }
                continue;
            }
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
        assert!(pre_capture.is_some(), "capture must succeed in a git repo");
        let pre_state = pre_capture.unwrap();

        run(dir.path()).unwrap();

        // Load the model from the serialized facts.json.
        let facts = dir.path().join(".codebro/facts.json");
        let model: FactsModel =
            serde_json::from_str(&std::fs::read_to_string(facts).unwrap()).unwrap();

        // The generation_repo_state must be Some and match the pre-generation capture.
        let gen_state = model
            .generation_repo_state()
            .expect("generation_repo_state must be set");
        assert_eq!(
            gen_state.working_tree_hash, pre_state.working_tree_hash,
            "generation_repo_state must reflect pre-generation repo state"
        );
        assert_eq!(
            gen_state.commit_sha, pre_state.commit_sha,
            "generation_repo_state must reflect pre-generation commit SHA"
        );
    }

    /// Regression: call edges must resolve their caller to a real symbol
    /// fact. The parser only knows the bare file name while symbol facts
    /// carry the workspace-relative path; when init failed to normalize
    /// this, every verified edge fell back to a synthetic `anon_call` id
    /// that resolved to no fact, and store validation reported one
    /// `broken_index` issue per relationship.
    #[test]
    fn call_edges_resolve_to_real_symbols_and_store_validates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"edge-probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "mod util;\nfn aaa() {}\npub fn main() { aaa(); util::helper(); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/util.rs"),
            "pub fn helper() -> i32 { 42 }\n",
        )
        .unwrap();

        run(dir.path()).unwrap();
        let model: FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).unwrap(),
        )
        .unwrap();

        // Every Calls edge must have both endpoints resolving to real
        // symbol facts — no dangling synthetic ids.
        let sym_ids: std::collections::HashSet<&str> =
            model.symbols().iter().map(|s| s.id.as_str()).collect();
        let mod_ids: std::collections::HashSet<&str> =
            model.modules().iter().map(|m| m.id.as_str()).collect();
        assert!(
            !model.relationships().is_empty(),
            "expected AST-derived relationships"
        );
        for r in model.relationships() {
            let endpoint_ok = |e: &crate::engineering_facts::FactId| match e {
                crate::engineering_facts::FactId::Symbol(s) => sym_ids.contains(s.as_str()),
                crate::engineering_facts::FactId::Module(m) => mod_ids.contains(m.as_str()),
                _ => false,
            };
            let src = format!("{:?}", r.source);
            let tgt = format!("{:?}", r.target);
            assert!(
                !src.contains("anon_call") && !tgt.contains("anon_call"),
                "dangling anon_call endpoint in edge {}: {} -> {}",
                r.id,
                src,
                tgt
            );
            assert!(endpoint_ok(&r.source), "unresolved source in {}", r.id);
            assert!(endpoint_ok(&r.target), "unresolved target in {}", r.id);
        }

        // The caller of both calls must be attributed to their enclosing
        // function (`main`), matched by name + workspace-relative path.
        let main_id = model
            .symbols()
            .iter()
            .find(|s| s.name == "main")
            .map(|s| s.id.as_str().to_string())
            .expect("main symbol fact");
        let call_targets_from_main: Vec<String> = model
            .relationships()
            .iter()
            .filter(|r| {
                r.kind == crate::engineering_facts::RelationshipKind::Calls
                    && matches!(&r.source, crate::engineering_facts::FactId::Symbol(s) if s.as_str() == main_id)
            })
            .map(|r| format!("{:?}", r.target))
            .collect();
        assert!(
            call_targets_from_main.iter().any(|t| t.contains("aaa")),
            "main -> aaa edge missing, got {call_targets_from_main:?}"
        );
        assert!(
            call_targets_from_main.iter().any(|t| t.contains("helper")),
            "main -> helper (qualified call) edge missing, got {call_targets_from_main:?}"
        );

        // The frozen store must validate with zero issues.
        let report = crate::fact_store::store::FactStore::build(model.clone()).validate();
        assert!(
            report.passed(),
            "store validation failed: {:?}",
            report.issues.iter().take(5).collect::<Vec<_>>()
        );
        assert_eq!(report.issue_count(), 0);
    }

    /// Regression: import edges must resolve now that import locations use
    /// the workspace-relative path (`find_module_for_file` compares against
    /// module paths keyed by that same relative path).
    #[test]
    fn ast_import_edges_resolve_between_modules() {        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"import-probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "mod util;\nuse crate::util;\nfn main() { util::helper(); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/util.rs"),
            "pub fn helper() {}\n",
        )
        .unwrap();

        run(dir.path()).unwrap();
        let model: FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).unwrap(),
        )
        .unwrap();

        let imports: Vec<_> = model
            .relationships()
            .iter()
            .filter(|r| r.kind == crate::engineering_facts::RelationshipKind::Imports)
            .collect();
        assert!(!imports.is_empty(), "expected AST-derived Imports edges");
        let mod_ids: std::collections::HashSet<&str> =
            model.modules().iter().map(|m| m.id.as_str()).collect();
        for r in &imports {
            let ok = |e: &crate::engineering_facts::FactId| match e {
                crate::engineering_facts::FactId::Module(m) => mod_ids.contains(m.as_str()),
                _ => false,
            };
            assert!(
                ok(&r.source) && ok(&r.target),
                "unresolved import edge {}",
                r.id
            );
        }
    }

    /// init must populate the project identity from the deterministic
    /// workspace surface: description, toolchain, frameworks, architecture
    /// summary and important files.
    #[test]
    fn init_populates_project_identity_from_workspace_surface() {        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"goal-probe\"\ndescription = \"Probe whether init fills identity.\"\n\n[dependencies]\ntokio = \"1\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();

        run(dir.path()).unwrap();
        let raw =
            std::fs::read_to_string(dir.path().join(".codebro/project_identity.json")).unwrap();
        let identity: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(
            identity["description"],
            "Probe whether init fills identity.",
            "manifest description flows into identity"
        );
        assert_eq!(identity["build_system"], "cargo");
        assert_eq!(identity["testing_framework"], "cargo test");
        assert!(identity["frameworks"].as_array().unwrap().iter().any(|f| f == "tokio"));
        assert!(!identity["architecture_summary"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        assert!((identity["known_modules"].as_array().unwrap()).len() >= 1);
        assert!(identity["important_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "Cargo.toml"));
    }

    /// Re-running init must never overwrite authored identity data
    /// (goals, constraints, decisions) — inference only fills gaps.
    #[test]
    fn init_preserves_authored_identity_on_reinit() {
        use crate::project_identity::{IdentityChanges, ProjectIdentityUpdater};

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"merge-probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();

        // First init: inference fills the gaps.
        run(dir.path()).unwrap();

        // Author goals on top of the inferred identity.
        let mut runtime = crate::project_identity::ProjectIdentityRuntime::new(dir.path());
        runtime.load().unwrap();
        let current = runtime.snapshot();
        let mut changes = IdentityChanges::new();
        changes.set_description = Some("Human-authored purpose statement".to_string());
        changes.add_constraints = vec!["never touch release tags".to_string()];
        let mut updater = ProjectIdentityUpdater::new(dir.path());
        let result = updater.update(&current, changes).expect("update applies");
        assert!(result.applied);

        // Re-init: authored values survive; nothing is clobbered.
        run(dir.path()).unwrap();
        let raw =
            std::fs::read_to_string(dir.path().join(".codebro/project_identity.json")).unwrap();
        let identity: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            identity["description"],
            "Human-authored purpose statement",
            "authored description must win over re-inference"
        );
        assert_eq!(identity["build_system"], "cargo", "inferred fields persist");
        assert!(identity["known_constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "never touch release tags"));
    }

    /// Documentation mining: ADRs become engineering decisions, an
    /// AGENTS.md Conventions section becomes coding conventions, and
    /// CHANGELOG releases become milestones. Re-running init must be
    /// idempotent — no duplicated mined entries.
    #[test]
    fn init_mines_documented_intent_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("docs/ADR")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"mine-probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        std::fs::write(
            dir.path().join("docs/ADR/ADR-001-pick-sqlite.md"),
            "# ADR-001: Pick SQLite\n\n**Status:** Accepted\n\n## Context\nSingle-file embedded storage is enough for v1.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "## Dev workflow\n\n### Conventions\n\n- Format before commit.\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("CHANGELOG.md"), "## [0.1.0] - 2026-01-01\n- first\n").unwrap();

        run(dir.path()).unwrap();
        let read_identity = || {
            let raw =
                std::fs::read_to_string(dir.path().join(".codebro/project_identity.json")).unwrap();
            let identity: serde_json::Value = serde_json::from_str(&raw).unwrap();
            identity
        };
        let identity = read_identity();
        assert_eq!(
            identity["engineering_decisions"][0]["id"], "adr-001-pick-sqlite",
            "ADR decision mined"
        );
        assert_eq!(identity["engineering_decisions"][0]["status"], "Accepted");
        assert!(identity["coding_conventions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "Format before commit."));
        assert_eq!(identity["recent_milestones"].as_array().unwrap().len(), 1);

        // Idempotent: second init must not duplicate anything.
        run(dir.path()).unwrap();
        let identity = read_identity();
        assert_eq!(identity["engineering_decisions"].as_array().unwrap().len(), 1);
        assert_eq!(
            identity["coding_conventions"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| *c == "Format before commit.")
                .count(),
            1
        );
        assert_eq!(identity["recent_milestones"].as_array().unwrap().len(), 1);
    }

    /// Regression: machine-generated source files that embed datasets
    /// (hundreds of MB of array literals with a .py/.js extension) must be
    /// skipped, not read and parsed — parsing them exploded memory (~700 MB
    /// RSS for a single 9 MB file) and CPU until the process appeared to
    /// hang on dataset-heavy repos.
    #[test]
    fn init_skips_oversized_generated_source_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"bigdata-probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        // Small, legitimate source: parsed normally.
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn real_fn() -> u32 { 1 }\n").unwrap();
        // Oversized generated "source": 600 KiB of embedded data.
        let big = format!("pub const BLOB: &[u8] = &[\n{}\n];\n", "1,".repeat(300_000));
        std::fs::write(dir.path().join("src/embedded_data.rs"), big).unwrap();

        run(dir.path()).unwrap();
        let model: FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).unwrap(),
        )
        .unwrap();

        let names: Vec<&str> = model.symbols().iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"real_fn"), "small file must still parse");
        assert!(
            !model.modules().iter().any(|m| m.path.as_deref() == Some("src/embedded_data.rs")),
            "oversized file must not produce a module fact"
        );
    }

    /// Receiver-type resolution: `A::new()` and `B::new()` must resolve to
    /// their own impl's method even though the bare name is ambiguous, and
    /// `self.ping_hit()` inside `impl Ping` must pick Ping's method.
    #[test]
    fn receiver_type_disambiguates_same_name_methods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"recv-probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub struct Ping;\npub struct Pong;\nimpl Ping { pub fn new() -> Self { Ping } pub fn ping_hit(&self) {} }\nimpl Pong { pub fn new() -> Self { Pong } pub fn pong_hit(&self) {} }\npub fn mainish() { let _p = Ping::new(); let _q = Pong::new(); }\nimpl Ping { pub fn go(&self) { self.ping_hit(); } }\n",
        )
        .unwrap();

        run(dir.path()).unwrap();
        let model: FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).unwrap(),
        )
        .unwrap();

        let name_of_id: std::collections::HashMap<&str, &str> = model
            .symbols()
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        let endpoint_name = |e: &crate::engineering_facts::FactId| match e {
            crate::engineering_facts::FactId::Symbol(s) => name_of_id.get(s.as_str()).copied(),
            _ => None,
        };

        let calls: Vec<(String, String)> = model
            .relationships()
            .iter()
            .filter(|r| r.kind == crate::engineering_facts::RelationshipKind::Calls)
            .filter_map(|r| {
                Some((
                    endpoint_name(&r.source)?.to_string(),
                    endpoint_name(&r.target)?.to_string(),
                ))
            })
            .collect();

        // Both same-name `new` methods resolved, each exactly once.
        let news: Vec<&String> = calls
            .iter()
            .filter(|(s, t)| s == "mainish" && t == "new")
            .map(|(_, t)| t)
            .collect();
        assert_eq!(
            news.len(),
            2,
            "mainish must reach both new() impls, got {calls:?}"
        );

        // Same-line calls to same-named methods must not produce
        // duplicate relationship ids (id carries a target hash).
        {
            let raw =
                std::fs::read_to_string(dir.path().join(".codebro/facts.json")).unwrap();
            let model: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let ids: Vec<&str> = model["relationships"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|r| r["id"].as_str())
                .collect();
            let mut sorted = ids.clone();
            sorted.sort_unstable();
            let dupes = sorted.windows(2).filter(|w| w[0] == w[1]).count();
            assert_eq!(dupes, 0, "duplicate relationship ids: {ids:?}");
        }

        // Self-call inside impl Ping resolves to Ping's method only.
        assert!(
            calls.iter().any(|(s, t)| s == "go" && t == "ping_hit"),
            "self.ping_hit() must resolve within impl Ping, got {calls:?}"
        );
        assert!(
            !calls.iter().any(|(_, t)| t == "pong_hit"),
            "no edge may invent a call to pong_hit, got {calls:?}"
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



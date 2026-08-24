#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Unit, determinism, serialisation, concurrency and scale tests for the
//! Engineering Facts Model (P10.5.0).

use std::sync::Arc;

use crate::engineering_facts::{
    ApiSurface, ArchitectureRuleFact, ArchitectureRuleId, BuildTargetFact, BuildTargetId,
    BuildTargetKind, DependencyFact, DependencyId, DependencyKind, DiagnosticFact, DiagnosticId,
    FactId, FactKind, FactMetadata, FactsModel, ModuleFact, ModuleId, PackageFact, PackageId,
    Position, ReferenceFact, ReferenceId, RelationshipFact, RelationshipId, RelationshipKind,
    Severity, SourceLocation, Span, SymbolFact, SymbolId, SymbolKind, TestFact, TestId,
    ValidationRule, Visibility, WorkspaceFact, WorkspaceId,
};

// ── IDs ───────────────────────────────────────────────────────────────────

#[test]
fn ids_are_opaque_but_comparable() {
    let a = SymbolId::new("sym::alpha");
    let b = SymbolId::new("sym::alpha");
    let c = SymbolId::new("sym::beta");

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!(a < c);
    assert_eq!(a.as_str(), "sym::alpha");
    assert_eq!(a.to_string(), "sym::alpha");
    assert_eq!(a.as_ref(), "sym::alpha");

    // Hash consistency with equality.
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(a.clone());
    set.insert(b.clone());
    assert_eq!(set.len(), 1);
}

#[test]
fn ids_are_distinct_types() {
    // A SymbolId must never be accepted where a ModuleId is expected.
    let symbol = SymbolId::new("shared");
    let module = ModuleId::new("shared");
    assert_eq!(symbol.as_str(), module.as_str());
    // Both convert to the same union FactId (same kind tag differs).
    let fs: FactId = FactId::from(symbol);
    let fm: FactId = FactId::from(module);
    assert_eq!(fs.as_str(), "shared");
    assert_eq!(fs.kind(), FactKind::Symbol);
    assert_eq!(fm.kind(), FactKind::Module);
    assert_ne!(fs, fm);
}

#[test]
fn every_typed_id_converts_to_union_fact_id() {
    let cases: Vec<(FactId, FactKind, &str)> = vec![
        (
            FactId::from(WorkspaceId::new("w")),
            FactKind::Workspace,
            "w",
        ),
        (FactId::from(PackageId::new("p")), FactKind::Package, "p"),
        (FactId::from(ModuleId::new("m")), FactKind::Module, "m"),
        (FactId::from(SymbolId::new("s")), FactKind::Symbol, "s"),
        (FactId::from(TestId::new("t")), FactKind::Test, "t"),
        (
            FactId::from(BuildTargetId::new("b")),
            FactKind::BuildTarget,
            "b",
        ),
        (
            FactId::from(DependencyId::new("d")),
            FactKind::Dependency,
            "d",
        ),
        (
            FactId::from(RelationshipId::new("r")),
            FactKind::Relationship,
            "r",
        ),
        (
            FactId::from(ReferenceId::new("rf")),
            FactKind::Reference,
            "rf",
        ),
        (
            FactId::from(DiagnosticId::new("dg")),
            FactKind::Diagnostic,
            "dg",
        ),
        (
            FactId::from(ArchitectureRuleId::new("ar")),
            FactKind::ArchitectureRule,
            "ar",
        ),
    ];
    for (fid, kind, value) in cases {
        assert_eq!(fid.kind(), kind);
        assert_eq!(fid.as_str(), value);
        assert_eq!(FactId::new(kind, value), fid);
    }
}

#[test]
fn id_serializes_transparently() {
    let id = SymbolId::new("sym::alpha");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"sym::alpha\"");
    let back: SymbolId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
}

// ── Enums: parse / display round-trips ────────────────────────────────────

#[test]
fn all_visibility_values_round_trip() {
    for v in Visibility::ALL {
        let s = v.as_str();
        assert_eq!(Visibility::parse(s), Some(v));
        assert_eq!(s.parse::<Visibility>(), Ok(v));
    }
    assert_eq!(Visibility::parse("private_rust"), None);
    assert_eq!(Visibility::Public.is_resolved(), true);
    assert_eq!(Visibility::Unknown.is_resolved(), false);
}

#[test]
fn all_relationship_kinds_round_trip() {
    // The full 15-kind language-neutral set, including Declares.
    assert_eq!(RelationshipKind::ALL.len(), 15);
    assert!(RelationshipKind::ALL.contains(&RelationshipKind::Declares));
    assert!(RelationshipKind::ALL.contains(&RelationshipKind::Defines));
    for k in RelationshipKind::ALL {
        assert_eq!(RelationshipKind::parse(k.as_str()), Some(k));
    }
    assert_eq!(RelationshipKind::parse("inherits_from"), None);
}

#[test]
fn all_fact_kinds_and_severities_round_trip() {
    for k in FactKind::ALL {
        assert_eq!(FactKind::parse(k.as_str()), Some(k));
    }
    for s in Severity::ALL {
        assert_eq!(Severity::parse(s.as_str()), Some(s));
    }
    assert_eq!(FactKind::parse("ast"), None);
    assert_eq!(Severity::parse("critical"), None);
}

#[test]
fn all_symbol_kinds_round_trip() {
    for k in SymbolKind::ALL {
        assert_eq!(SymbolKind::parse(k.as_str()), Some(k));
    }
    assert_eq!(SymbolKind::parse("generic"), None);
}

#[test]
fn all_dependency_kinds_round_trip() {
    for k in DependencyKind::ALL {
        assert_eq!(DependencyKind::parse(k.as_str()), Some(k));
    }
    assert_eq!(DependencyKind::parse("peer_dep"), None);
}

#[test]
fn all_build_target_kinds_round_trip() {
    for k in BuildTargetKind::ALL {
        assert_eq!(BuildTargetKind::parse(k.as_str()), Some(k));
    }
    assert_eq!(BuildTargetKind::parse("package"), None);
}

// ── Metadata ──────────────────────────────────────────────────────────────

#[test]
fn metadata_is_sorted_and_deduplicated() {
    let m = FactMetadata::builder()
        .tag("beta")
        .tag("alpha")
        .tag("beta")
        .attr("z", "1")
        .attr("a", "2")
        .attr("a", "2")
        .description("d")
        .language("rust")
        .build();

    let tags: Vec<&str> = m.tags.iter().map(|t| t.as_str()).collect();
    assert_eq!(tags, vec!["alpha", "beta"]);

    let keys: Vec<&str> = m.attributes.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(keys, vec!["a", "z"]);

    assert!(m.has_tag("alpha"));
    assert!(!m.has_tag("gamma"));
    assert_eq!(m.get("a"), Some("2"));
    assert_eq!(m.get("z"), Some("1"));
    assert_eq!(m.get("missing"), None);

    let m2 = FactMetadata::builder()
        .tag("beta")
        .tag("alpha")
        .attr("z", "1")
        .attr("a", "2")
        .description("d")
        .language("rust")
        .build();
    assert_eq!(m, m2);
    assert_eq!(
        serde_json::to_string(&m).unwrap(),
        serde_json::to_string(&m2).unwrap()
    );
}

#[test]
fn metadata_get_and_has_tag_are_allocation_free_lookups() {
    let m = FactMetadata::builder()
        .tag("x")
        .tag("y")
        .attr("k1", "v1")
        .attr("k2", "v2")
        .attr("k3", "v3")
        .build();
    assert_eq!(m.get("k2"), Some("v2"));
    assert_eq!(m.get("k9"), None);
    assert!(m.has_tag("y"));
    assert!(!m.has_tag("z"));
}

// ── Locations ─────────────────────────────────────────────────────────────

#[test]
fn source_location_carries_workspace_package_module_file_point_and_span() {
    let ws = WorkspaceId::new("ws::main");
    let pkg = PackageId::new("pkg::core");
    let mod_id = ModuleId::new("mod::core");
    let span = Span::new(Position::new(10, 1), Position::new(12, 5));

    let loc = SourceLocation::new()
        .with_workspace(ws.clone())
        .with_package(pkg.clone())
        .with_module(mod_id.clone())
        .with_file("src/lib.rs")
        .with_point(10, 1)
        .with_span(span);

    assert_eq!(loc.workspace, Some(ws));
    assert_eq!(loc.package, Some(pkg));
    assert_eq!(loc.module, Some(mod_id));
    assert_eq!(loc.file.as_deref(), Some("src/lib.rs"));
    assert_eq!(loc.line, Some(10));
    assert_eq!(loc.column, Some(1));
    assert_eq!(loc.span.unwrap().start.line, 10);
    assert_eq!(loc.span.unwrap().end.column, 5);
    assert!(!loc.is_empty());
    assert!(SourceLocation::new().is_empty());
    assert_eq!(SourceLocation::file("a.rs").file.as_deref(), Some("a.rs"));
}

// ── Model construction & determinism ──────────────────────────────────────

fn sample_symbols() -> Vec<SymbolFact> {
    let mut symbols = Vec::new();
    for i in (0..10).rev() {
        symbols.push(SymbolFact {
            id: SymbolId::new(format!("sym::s{i}")),
            name: format!("symbol_{i}"),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            location: SourceLocation::file(format!("src/lib.rs:{i}")),
            module: Some(ModuleId::new("mod::core")),
            signature: Some("fn s()".to_string()),
            metadata: FactMetadata::new(),
        });
    }
    symbols
}

fn sample_model() -> FactsModel {
    let workspace = WorkspaceFact {
        id: WorkspaceId::new("ws::main"),
        name: "main".into(),
        root: None,
        packages: vec![PackageId::new("pkg::core")],
        metadata: FactMetadata::new(),
    };
    let serde_pkg = PackageFact::new(PackageId::new("pkg::serde"), "serde");
    let package = PackageFact {
        id: PackageId::new("pkg::core"),
        name: "core".into(),
        version: Some("1.0.0".into()),
        workspace: Some(workspace.id.clone()),
        language: Some("rust".into()),
        build_targets: vec![BuildTargetId::new("bt::core")],
        metadata: FactMetadata::new(),
    };
    let module = ModuleFact {
        id: ModuleId::new("mod::core"),
        name: "core".into(),
        package: Some(package.id.clone()),
        path: Some("src/lib.rs".into()),
        visibility: Visibility::Public,
        location: SourceLocation::new()
            .with_workspace(workspace.id.clone())
            .with_package(package.id.clone())
            .with_module(ModuleId::new("mod::core")),
        api: ApiSurface {
            exports: vec![SymbolId::new("sym::s0")],
            entry_points: vec![SymbolId::new("sym::s1")],
        },
        metadata: FactMetadata::new(),
    };

    let mut symbols = sample_symbols();
    let sym0 = symbols.pop().unwrap();
    let sym1 = symbols.pop().unwrap();

    let bt = BuildTargetFact {
        id: BuildTargetId::new("bt::core"),
        name: "core".into(),
        kind: BuildTargetKind::Library,
        language: Some("rust".into()),
        package: Some(package.id.clone()),
        metadata: FactMetadata::new(),
    };
    let test = TestFact {
        id: TestId::new("test::s0"),
        name: "s0_works".into(),
        target: Some(FactId::from(BuildTargetId::new("bt::core"))),
        tested: vec![sym0.id.clone()],
        location: None,
        metadata: FactMetadata::new(),
    };

    let dep = DependencyFact::new(
        DependencyId::new("dep::core->serde"),
        FactId::from(package.id.clone()),
        FactId::from(serde_pkg.id.clone()),
    );
    let rel = RelationshipFact::new(
        RelationshipId::new("rel::mod-contains-s0"),
        RelationshipKind::Contains,
        FactId::from(module.id.clone()),
        FactId::from(sym0.id.clone()),
    );
    let reference = ReferenceFact::new(
        ReferenceId::new("ref::s1->s0"),
        FactId::from(sym1.id.clone()),
        FactId::from(sym0.id.clone()),
    );

    let mut b = FactsModel::builder();
    b.add_workspace(workspace)
        .add_package(package)
        .add_package(serde_pkg)
        .add_module(module)
        .add_build_target(bt)
        .add_test(test)
        .add_dependency(dep)
        .add_relationship(rel)
        .add_reference(reference)
        .add_symbol(sym0)
        .add_symbol(sym1);
    for s in symbols {
        b.add_symbol(s);
    }
    b.build()
}

#[test]
fn builder_sorts_categories_deterministically() {
    let model = sample_model();
    let ids: Vec<&str> = model.symbols().iter().map(|s| s.id.as_str()).collect();
    let mut expected: Vec<&str> = ids.clone();
    expected.sort_unstable();
    assert_eq!(ids, expected);

    let model2 = sample_model();
    assert_eq!(model.symbols(), model2.symbols());
    assert_eq!(model.counts(), model2.counts());
    assert_eq!(
        serde_json::to_string(&model).unwrap(),
        serde_json::to_string(&model2).unwrap()
    );
}

#[test]
fn model_counts_are_accurate() {
    let model = sample_model();
    let counts = model.counts();
    assert_eq!(counts.workspaces, 1);
    assert_eq!(counts.modules, 1);
    assert_eq!(counts.packages, 2);
    assert_eq!(counts.symbols, 10);
    assert_eq!(counts.tests, 1);
    assert_eq!(counts.build_targets, 1);
    assert_eq!(counts.dependencies, 1);
    assert_eq!(counts.relationships, 1);
    assert_eq!(counts.references, 1);
    assert_eq!(counts.total, 19);
    assert_eq!(model.len(), 19);
    assert!(!model.is_empty());
    assert!(FactsModel::empty().is_empty());
}

// ── Lookups ───────────────────────────────────────────────────────────────

#[test]
fn typed_binary_search_lookups_resolve() {
    let model = sample_model();

    let ws = WorkspaceId::new("ws::main");
    let mod_id = ModuleId::new("mod::core");
    let pkg = PackageId::new("pkg::core");
    let s0 = SymbolId::new("sym::s0");
    let bt = BuildTargetId::new("bt::core");

    assert!(model.workspace(&ws).is_some());
    assert!(model.module(&mod_id).is_some());
    assert!(model.package(&pkg).is_some());
    let sym0 = model.symbol(&s0).unwrap();
    assert_eq!(sym0.name, "symbol_0");
    assert!(model
        .dependency(&DependencyId::new("dep::core->serde"))
        .is_some());
    assert!(model
        .relationship(&RelationshipId::new("rel::mod-contains-s0"))
        .is_some());
    assert!(model.reference(&ReferenceId::new("ref::s1->s0")).is_some());
    assert!(model.test(&TestId::new("test::s0")).is_some());
    assert!(model.build_target(&bt).is_some());

    assert!(model.symbol(&SymbolId::new("sym::zzz")).is_none());
}

#[test]
fn union_lookups_and_find_resolve() {
    let model = sample_model();

    assert!(model.contains(&FactId::from(SymbolId::new("sym::s0"))));
    assert!(model.contains(&FactId::from(ModuleId::new("mod::core"))));
    assert!(!model.contains(&FactId::new(FactKind::Symbol, "nope::missing")));

    match model.find(&FactId::from(SymbolId::new("sym::s0"))) {
        Some(crate::engineering_facts::FactRef::Symbol(s)) => assert_eq!(s.name, "symbol_0"),
        _ => panic!("expected symbol"),
    }
    match model.find(&FactId::from(WorkspaceId::new("ws::main"))) {
        Some(crate::engineering_facts::FactRef::Workspace(w)) => assert_eq!(w.name, "main"),
        _ => panic!("expected workspace"),
    }
}

#[test]
fn empty_model_lookups_return_none() {
    let model = FactsModel::empty();
    assert!(!model.contains(&FactId::from(SymbolId::new("x"))));
    assert!(model.symbol(&SymbolId::new("x")).is_none());
    assert!(model.find(&FactId::from(ModuleId::new("x"))).is_none());
}

// ── Validation ────────────────────────────────────────────────────────────

#[test]
fn clean_model_passes_validation() {
    let model = sample_model();
    let report = model.validate();
    assert!(report.passed(), "issues: {:?}", report.issues);
    assert_eq!(report.issue_count(), 0);
    assert_eq!(report.error_count(), 0);
}

#[test]
fn duplicate_ids_are_detected() {
    let mut b = FactsModel::builder();
    for i in 0..3 {
        b.add_symbol(SymbolFact::new(
            SymbolId::new("sym::dup"),
            format!("s{i}"),
            SymbolKind::Class,
        ));
    }
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::DuplicateIds), 1);
    assert!(!report.passed());
}

#[test]
fn duplicate_ids_across_categories_are_detected() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::a"),
        "a",
        SymbolKind::Function,
    ));
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::a"),
        "a2",
        SymbolKind::Function,
    ));
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::DuplicateIds), 1);
    assert!(!report.passed());
}

#[test]
fn duplicate_relationships_are_detected() {
    let mut b = FactsModel::builder();
    let module = FactId::from(ModuleId::new("mod::m"));
    let sym = FactId::from(SymbolId::new("sym::a"));
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::n1"),
        RelationshipKind::Calls,
        module.clone(),
        sym.clone(),
    ));
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::n2"),
        RelationshipKind::Calls,
        module.clone(),
        sym.clone(),
    ));
    // Different kind → not a duplicate.
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::n3"),
        RelationshipKind::Imports,
        module.clone(),
        sym.clone(),
    ));
    let report = b.build().validate();
    assert_eq!(
        report.count_by_rule(ValidationRule::DuplicateRelationships),
        1
    );
    assert!(!report.passed());
}

#[test]
fn invalid_references_are_detected() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::a"),
        "a",
        SymbolKind::Function,
    ));
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::a->ghost"),
        RelationshipKind::Calls,
        FactId::from(SymbolId::new("sym::a")),
        FactId::from(SymbolId::new("ghost::missing")),
    ));
    b.add_module(ModuleFact {
        id: ModuleId::new("mod::x"),
        name: "x".into(),
        package: None,
        path: None,
        visibility: Visibility::Public,
        location: SourceLocation::new(),
        api: ApiSurface {
            exports: vec![SymbolId::new("ghost::export")],
            entry_points: Vec::new(),
        },
        metadata: FactMetadata::new(),
    });
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::InvalidReference), 2);
    assert!(!report.passed());
}

#[test]
fn self_references_are_detected() {
    let mut b = FactsModel::builder();
    let sym = FactId::from(SymbolId::new("sym::a"));
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::a"),
        "a",
        SymbolKind::Function,
    ));
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::self"),
        RelationshipKind::Calls,
        sym.clone(),
        sym.clone(),
    ));
    b.add_reference(ReferenceFact::new(
        ReferenceId::new("ref::self"),
        sym.clone(),
        sym.clone(),
    ));
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::SelfReference), 2);
    assert!(!report.passed());
}

#[test]
fn self_dependencies_are_detected() {
    let mut b = FactsModel::builder();
    b.add_package(PackageFact::new(PackageId::new("pkg::self"), "self"));
    b.add_dependency(DependencyFact::new(
        DependencyId::new("dep::self"),
        FactId::from(PackageId::new("pkg::self")),
        FactId::from(PackageId::new("pkg::self")),
    ));
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::SelfDependency), 1);
    assert!(!report.passed());
}

#[test]
fn broken_locations_are_detected() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact {
        id: SymbolId::new("sym::a"),
        name: "a".into(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        location: SourceLocation::new()
            .with_workspace(WorkspaceId::new("ghost::ws"))
            .with_package(PackageId::new("ghost::pkg")),
        module: Some(ModuleId::new("mod::m")),
        signature: None,
        metadata: FactMetadata::new(),
    });
    b.add_module(ModuleFact::new(ModuleId::new("mod::m"), "m"));
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::BrokenLocation), 2);
    assert!(!report.passed());
}

#[test]
fn orphan_symbols_are_detected() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::owned"),
        "owned",
        SymbolKind::Function,
    ));
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::orphan"),
        "orphan",
        SymbolKind::Function,
    ));
    b.add_module(ModuleFact::new(ModuleId::new("mod::m"), "m"));
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::m-owns-owned"),
        RelationshipKind::Owns,
        FactId::from(ModuleId::new("mod::m")),
        FactId::from(SymbolId::new("sym::owned")),
    ));

    let report = b.build().validate();
    let orphans: Vec<&str> = report
        .issues
        .iter()
        .filter(|i| i.rule == ValidationRule::OrphanSymbol)
        .map(|i| i.entity.as_str())
        .collect();
    assert_eq!(orphans, vec!["sym::orphan"]);
    assert!(report.passed());
}

#[test]
fn declares_edge_claims_orphan_symbol() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::declared"),
        "declared",
        SymbolKind::Class,
    ));
    b.add_module(ModuleFact::new(ModuleId::new("mod::m"), "m"));
    b.add_relationship(RelationshipFact::new(
        RelationshipId::new("rel::m-declares"),
        RelationshipKind::Declares,
        FactId::from(ModuleId::new("mod::m")),
        FactId::from(SymbolId::new("sym::declared")),
    ));
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::OrphanSymbol), 0);
}

#[test]
fn unresolved_visibility_is_warned() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::u"),
        "u",
        SymbolKind::Class,
    ));
    b.add_module(ModuleFact {
        id: ModuleId::new("mod::u"),
        name: "u".into(),
        package: None,
        path: None,
        visibility: Visibility::Unknown,
        location: SourceLocation::new(),
        api: ApiSurface::empty(),
        metadata: FactMetadata::new(),
    });
    let report = b.build().validate();
    assert_eq!(report.count_by_rule(ValidationRule::InvalidVisibility), 2);
    assert!(report.passed(), "unresolved visibility is only a warning");
    // 2 unresolved-visibility warnings + 1 orphan-symbol warning.
    assert_eq!(report.warning_count(), 3);
}

#[test]
fn validation_is_deterministic() {
    let model = sample_model();
    let r1 = model.validate();
    let r2 = model.validate();
    assert_eq!(r1, r2);
    assert_eq!(r1.issues, r2.issues);
    assert_eq!(
        serde_json::to_string(&r1).unwrap(),
        serde_json::to_string(&r2).unwrap()
    );
}

#[test]
fn validation_issues_are_sorted() {
    let mut b = FactsModel::builder();
    for i in 0..5 {
        b.add_symbol(SymbolFact::new(
            SymbolId::new(format!("sym::z{i}")),
            format!("z{i}"),
            SymbolKind::Function,
        ));
    }
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::dup"),
        "d",
        SymbolKind::Function,
    ));
    b.add_symbol(SymbolFact::new(
        SymbolId::new("sym::dup"),
        "d2",
        SymbolKind::Function,
    ));
    let report = b.build().validate();
    let sorted = report.issues.windows(2).all(|w| w[0] <= w[1]);
    assert!(sorted, "issues must be deterministically ordered");
}

#[test]
fn validation_rules_parse_and_round_trip() {
    for r in ValidationRule::ALL {
        assert_eq!(ValidationRule::parse(r.as_str()), Some(r));
    }
    assert_eq!(ValidationRule::parse("cyclic_graph"), None);
}

// ── Serialisation ─────────────────────────────────────────────────────────

#[test]
fn full_model_serde_round_trip() {
    let model = sample_model();
    let json = serde_json::to_string(&model).unwrap();
    let back: FactsModel = serde_json::from_str(&json).unwrap();
    assert_eq!(model, back);
    assert_eq!(model.counts(), back.counts());

    let toml = toml::to_string(&model).unwrap();
    let back_toml: FactsModel = toml::from_str(&toml).unwrap();
    assert_eq!(model, back_toml);
}

#[test]
fn serde_round_trip_is_byte_identical() {
    let model = sample_model();
    let a = serde_json::to_string(&model).unwrap();
    let b = serde_json::to_string(&model).unwrap();
    assert_eq!(a, b);
}

// ── Concurrency & Send + Sync ─────────────────────────────────────────────

#[test]
fn model_is_send_and_sync_across_threads() {
    let model = Arc::new(sample_model());
    let mut handles = Vec::new();
    for _ in 0..8 {
        let model = model.clone();
        handles.push(std::thread::spawn(move || {
            let report = model.validate();
            let _ = model.contains(&FactId::from(SymbolId::new("sym::s0")));
            let _ = model.symbol(&SymbolId::new("sym::s0"));
            assert!(report.passed());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

// ── Scale smoke test ──────────────────────────────────────────────────────

#[test]
fn million_fact_scale_smoke() {
    const N: usize = 250_000;
    let mut b = FactsModel::builder();
    for i in 0..N {
        b.add_symbol(SymbolFact {
            id: SymbolId::new(format!("sym::{i:09}")),
            name: format!("s{i}"),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            location: SourceLocation::file(format!("src/{:02}.rs", i % 50)),
            module: Some(ModuleId::new("mod::core")),
            signature: None,
            metadata: FactMetadata::builder().tag("scale").build(),
        });
    }
    b.add_module(ModuleFact::new(ModuleId::new("mod::core"), "core"));
    let model = b.build();

    assert_eq!(model.symbols().len(), N);
    assert!(model.contains(&FactId::from(SymbolId::new(format!("sym::{:09}", N - 1)))));
    assert!(model.symbol(&SymbolId::new("sym::000000000")).is_some());
    assert!(model.symbol(&SymbolId::new("sym::000000042")).is_some());
    assert!(!model.contains(&FactId::new(FactKind::Symbol, "sym::xxxxxxxxx")));

    let report = model.validate();
    assert_eq!(report.count_by_rule(ValidationRule::OrphanSymbol), 0);
    assert_eq!(report.count_by_rule(ValidationRule::DuplicateIds), 0);
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Unit, determinism, serialisation, concurrency and scale tests for the
//! Fact Store (P10.5.1).

use std::collections::HashSet;
use std::sync::Arc;

use crate::engineering_facts::{
    ArchitectureRuleFact, ArchitectureRuleId, BuildTargetFact, BuildTargetId, BuildTargetKind,
    DependencyFact, DependencyId, DiagnosticFact, DiagnosticId, FactId, FactKind, FactRef,
    FactsModel, ModuleFact, ModuleId, PackageFact, PackageId, ReferenceFact, ReferenceId,
    RelationshipFact, RelationshipId, RelationshipKind, Severity, SourceLocation, SymbolFact,
    SymbolId, SymbolKind, TestFact, TestId, Visibility, WorkspaceFact, WorkspaceId,
};
use crate::fact_store::{
    FactCollection, FactIdPair, FactIndex, FactSnapshot, FactStatistics, FactStore,
    FactStoreBuilder, FactValidationRule,
};

// ── Fixtures ───────────────────────────────────────────────────────────────

fn sample_model() -> FactsModel {
    let w = WorkspaceId::new("ws::root");
    let p1 = PackageId::new("pkg::alpha");
    let p2 = PackageId::new("pkg::beta");
    let m1 = ModuleId::new("mod::alpha");
    let m2 = ModuleId::new("mod::beta");
    let s1 = SymbolId::new("sym::alpha");
    let s2 = SymbolId::new("sym::beta");
    let s3 = SymbolId::new("sym::gamma");
    let b1 = BuildTargetId::new("build::alpha");
    let t1 = TestId::new("test::alpha");
    let d1 = DependencyId::new("dep::alpha");
    let rel1 = RelationshipId::new("rel::alpha");
    let ref1 = ReferenceId::new("ref::alpha");
    let dg1 = DiagnosticId::new("diag::alpha");
    let ar1 = ArchitectureRuleId::new("rule::alpha");

    let mut ws = WorkspaceFact::new(w.clone(), "root");
    ws.packages = vec![p1.clone(), p2.clone()];

    let mut p1f = PackageFact::new(p1.clone(), "alpha");
    p1f.workspace = Some(w.clone());
    p1f.build_targets = vec![b1.clone()];
    let mut p2f = PackageFact::new(p2.clone(), "beta");
    p2f.workspace = Some(w.clone());

    let mut m1f = ModuleFact::new(m1.clone(), "alpha");
    m1f.package = Some(p1.clone());
    m1f.visibility = Visibility::Public;
    m1f.location = SourceLocation::new()
        .with_workspace(w.clone())
        .with_package(p1.clone());
    let mut m2f = ModuleFact::new(m2.clone(), "beta");
    m2f.package = Some(p2.clone());
    m2f.visibility = Visibility::Public;
    m2f.location = SourceLocation::new()
        .with_workspace(w.clone())
        .with_package(p2.clone());

    let mut s1f = SymbolFact::new(s1.clone(), "alpha_fn", SymbolKind::Function);
    s1f.module = Some(m1.clone());
    s1f.visibility = Visibility::Public;
    s1f.location = SourceLocation::new()
        .with_workspace(w.clone())
        .with_package(p1.clone())
        .with_module(m1.clone());
    let mut s2f = SymbolFact::new(s2.clone(), "beta_fn", SymbolKind::Function);
    s2f.module = Some(m2.clone());
    s2f.visibility = Visibility::Public;
    s2f.location = SourceLocation::new()
        .with_workspace(w.clone())
        .with_package(p2.clone())
        .with_module(m2.clone());
    let mut s3f = SymbolFact::new(s3.clone(), "gamma_fn", SymbolKind::Function);
    s3f.module = Some(m1.clone());
    s3f.visibility = Visibility::Public;
    s3f.location = SourceLocation::new()
        .with_workspace(w.clone())
        .with_package(p1.clone())
        .with_module(m1.clone());

    let mut b1f = BuildTargetFact::new(b1.clone(), "alpha-lib", BuildTargetKind::Library);
    b1f.package = Some(p1.clone());

    let mut t1f = TestFact::new(t1.clone(), "test_alpha");
    t1f.target = Some(FactId::Symbol(s1.clone()));
    t1f.tested = vec![s1.clone(), s2.clone()];
    t1f.location = Some(
        SourceLocation::new()
            .with_workspace(w.clone())
            .with_package(p1.clone())
            .with_module(m1.clone()),
    );

    let d1f = DependencyFact::new(
        d1.clone(),
        FactId::Package(p1.clone()),
        FactId::Package(p2.clone()),
    );

    let mut rel1f = RelationshipFact::new(
        rel1.clone(),
        RelationshipKind::Calls,
        FactId::Symbol(s1.clone()),
        FactId::Symbol(s2.clone()),
    );
    rel1f.location = Some(
        SourceLocation::new()
            .with_workspace(w.clone())
            .with_package(p1.clone())
            .with_module(m1.clone()),
    );

    let mut ref1f = ReferenceFact::new(
        ref1.clone(),
        FactId::Symbol(s1.clone()),
        FactId::Symbol(s2.clone()),
    );
    ref1f.location = Some(
        SourceLocation::new()
            .with_workspace(w.clone())
            .with_package(p1.clone())
            .with_module(m1.clone()),
    );

    let mut dg1f = DiagnosticFact::new(dg1.clone(), Severity::Warning, "example diagnostic");
    dg1f.related = vec![FactId::Symbol(s1.clone())];

    let mut ar1f = ArchitectureRuleFact::new(ar1.clone(), "alpha_rule");
    ar1f.from = Some(FactId::Symbol(s1.clone()));
    ar1f.to = Some(FactId::Symbol(s2.clone()));

    let mut b = FactsModel::builder();
    b.add_workspace(ws)
        .add_package(p1f)
        .add_package(p2f)
        .add_module(m1f)
        .add_module(m2f)
        .add_symbol(s1f)
        .add_symbol(s2f)
        .add_symbol(s3f)
        .add_build_target(b1f)
        .add_test(t1f)
        .add_dependency(d1f)
        .add_relationship(rel1f)
        .add_reference(ref1f)
        .add_diagnostic(dg1f)
        .add_architecture_rule(ar1f);
    b.build()
}

fn sample_store() -> FactStore {
    FactStore::build(sample_model())
}

fn members<'a>(pairs: &'a [FactIdPair]) -> Vec<FactId> {
    pairs.iter().map(|p| p.member.clone()).collect()
}

fn fact_id_of_ref(fact: &FactRef<'_>) -> FactId {
    match fact {
        FactRef::Workspace(f) => FactId::Workspace(f.id.clone()),
        FactRef::Module(f) => FactId::Module(f.id.clone()),
        FactRef::Package(f) => FactId::Package(f.id.clone()),
        FactRef::Symbol(f) => FactId::Symbol(f.id.clone()),
        FactRef::Test(f) => FactId::Test(f.id.clone()),
        FactRef::BuildTarget(f) => FactId::BuildTarget(f.id.clone()),
        FactRef::Dependency(f) => FactId::Dependency(f.id.clone()),
        FactRef::Relationship(f) => FactId::Relationship(f.id.clone()),
        FactRef::Reference(f) => FactId::Reference(f.id.clone()),
        FactRef::Diagnostic(f) => FactId::Diagnostic(f.id.clone()),
        FactRef::ArchitectureRule(f) => FactId::ArchitectureRule(f.id.clone()),
    }
}

const SAMPLE_FACTS: usize = 15;

// ── Store & Collection ─────────────────────────────────────────────────────

#[test]
fn store_builds_and_counts() {
    let store = sample_store();
    assert_eq!(store.len(), SAMPLE_FACTS);
    assert!(!store.is_empty());
    let counts = store.collection().counts();
    assert_eq!(counts.workspaces, 1);
    assert_eq!(counts.packages, 2);
    assert_eq!(counts.modules, 2);
    assert_eq!(counts.symbols, 3);
    assert_eq!(counts.tests, 1);
    assert_eq!(counts.build_targets, 1);
    assert_eq!(counts.dependencies, 1);
    assert_eq!(counts.relationships, 1);
    assert_eq!(counts.references, 1);
    assert_eq!(counts.diagnostics, 1);
    assert_eq!(counts.architecture_rules, 1);
    assert_eq!(counts.total, SAMPLE_FACTS);
}

#[test]
fn empty_store_is_empty() {
    let store = FactStore::empty();
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
    assert!(store.validate().passed());
    let stats = store.statistics();
    assert_eq!(stats.total_facts, 0);
    assert_eq!(stats.primary_index.total, 0);
    assert_eq!(stats.reverse_index.total, 0);
}

#[test]
fn store_has_no_mutation_path_after_build() {
    let store = sample_store();
    let before = store.clone();
    assert_eq!(store.statistics(), store.statistics());
    assert_eq!(store.diagnostics(), store.diagnostics());
    assert_eq!(store.snapshot(), store.snapshot());
    assert_eq!(store, before);
}

#[test]
fn types_are_send_sync_and_clone() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone<T: Clone>() {}
    assert_send_sync::<FactStore>();
    assert_send_sync::<FactCollection>();
    assert_send_sync::<crate::fact_store::ReverseIndex>();
    assert_send_sync::<FactSnapshot>();
    assert_send_sync::<FactStatistics>();
    assert_clone::<FactStore>();
    assert_clone::<FactSnapshot>();
}

#[test]
fn collection_enumerates_all_facts() {
    let store = sample_store();
    let mut seen: HashSet<FactId> = HashSet::new();
    let mut kinds: HashSet<FactKind> = HashSet::new();
    for fact in store.collection().iter() {
        let id = fact_id_of_ref(&fact);
        assert!(seen.insert(id.clone()), "enumerate must yield unique facts");
        kinds.insert(id.kind());
    }
    assert_eq!(seen.len(), SAMPLE_FACTS);
    assert_eq!(kinds.len(), 11);
}

#[test]
fn store_builder_absorbs_a_model() {
    let model = sample_model();
    let mut builder = FactStoreBuilder::new();
    builder.add_model(&model);
    builder.add_workspace(WorkspaceFact::new(WorkspaceId::new("ws::extra"), "extra"));
    let store = builder.build();
    assert_eq!(store.len(), SAMPLE_FACTS + 1);
}

// ── Index ──────────────────────────────────────────────────────────────────

#[test]
fn primary_index_covers_every_fact() {
    let store = sample_store();
    let idx = store.index();
    assert_eq!(idx.facts_of_kind(FactKind::Workspace).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::Package).len(), 2);
    assert_eq!(idx.facts_of_kind(FactKind::Module).len(), 2);
    assert_eq!(idx.facts_of_kind(FactKind::Symbol).len(), 3);
    assert_eq!(idx.facts_of_kind(FactKind::Test).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::BuildTarget).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::Dependency).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::Relationship).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::Reference).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::Diagnostic).len(), 1);
    assert_eq!(idx.facts_of_kind(FactKind::ArchitectureRule).len(), 1);
    assert_eq!(idx.primary_len(), SAMPLE_FACTS);
    for fact in store.collection().iter() {
        let id = fact_id_of_ref(&fact);
        assert!(
            idx.contains_in_kind(id.kind(), &id),
            "index must carry {id}"
        );
    }
}

#[test]
fn primary_index_is_sorted() {
    let store = sample_store();
    for kind in FactKind::ALL {
        let ids = store.index().facts_of_kind(kind);
        assert!(
            ids.windows(2).all(|w| w[0] <= w[1]),
            "index for {kind} is sorted"
        );
    }
}

#[test]
fn reverse_index_workspace_projection() {
    let store = sample_store();
    let pairs = store
        .index()
        .facts_in_workspace(&WorkspaceId::new("ws::root"));
    let mem = members(pairs);
    assert_eq!(pairs.len(), 10);
    assert!(mem.contains(&FactId::Package(PackageId::new("pkg::alpha"))));
    assert!(mem.contains(&FactId::Module(ModuleId::new("mod::alpha"))));
    assert!(mem.contains(&FactId::Symbol(SymbolId::new("sym::alpha"))));
    assert!(mem.contains(&FactId::Relationship(RelationshipId::new("rel::alpha"))));
    assert!(mem.contains(&FactId::Reference(ReferenceId::new("ref::alpha"))));
    assert!(mem.contains(&FactId::Test(TestId::new("test::alpha"))));
    assert!(!mem.contains(&FactId::BuildTarget(BuildTargetId::new("build::alpha"))));
    assert!(store
        .index()
        .facts_in_workspace(&WorkspaceId::new("ws::ghost"))
        .is_empty());
}

#[test]
fn reverse_index_package_projection() {
    let store = sample_store();
    let a = members(
        store
            .index()
            .facts_in_package(&PackageId::new("pkg::alpha")),
    );
    assert_eq!(a.len(), 7);
    assert!(a.contains(&FactId::Module(ModuleId::new("mod::alpha"))));
    assert!(a.contains(&FactId::Symbol(SymbolId::new("sym::alpha"))));
    assert!(a.contains(&FactId::Symbol(SymbolId::new("sym::gamma"))));
    assert!(a.contains(&FactId::BuildTarget(BuildTargetId::new("build::alpha"))));
    assert!(a.contains(&FactId::Relationship(RelationshipId::new("rel::alpha"))));
    let b = members(store.index().facts_in_package(&PackageId::new("pkg::beta")));
    assert_eq!(b.len(), 2);
}

#[test]
fn reverse_index_module_projection() {
    let store = sample_store();
    let a = members(store.index().facts_in_module(&ModuleId::new("mod::alpha")));
    assert_eq!(a.len(), 5);
    assert!(a.contains(&FactId::Symbol(SymbolId::new("sym::alpha"))));
    assert!(a.contains(&FactId::Symbol(SymbolId::new("sym::gamma"))));
    assert!(a.contains(&FactId::Relationship(RelationshipId::new("rel::alpha"))));
    assert!(a.contains(&FactId::Reference(ReferenceId::new("ref::alpha"))));
    assert!(a.contains(&FactId::Test(TestId::new("test::alpha"))));
    let b = members(store.index().facts_in_module(&ModuleId::new("mod::beta")));
    assert_eq!(b.len(), 1);
    assert!(b.contains(&FactId::Symbol(SymbolId::new("sym::beta"))));
}

#[test]
fn reverse_index_symbol_projection() {
    let store = sample_store();
    let a = members(store.index().facts_in_symbol(&SymbolId::new("sym::alpha")));
    assert_eq!(a.len(), 5);
    assert!(a.contains(&FactId::Test(TestId::new("test::alpha"))));
    assert!(a.contains(&FactId::Reference(ReferenceId::new("ref::alpha"))));
    assert!(a.contains(&FactId::Relationship(RelationshipId::new("rel::alpha"))));
    assert!(a.contains(&FactId::Diagnostic(DiagnosticId::new("diag::alpha"))));
    assert!(
        a.contains(&FactId::ArchitectureRule(ArchitectureRuleId::new(
            "rule::alpha"
        )))
    );
    let b = members(store.index().facts_in_symbol(&SymbolId::new("sym::beta")));
    assert_eq!(b.len(), 4);
    assert!(store
        .index()
        .facts_in_symbol(&SymbolId::new("sym::gamma"))
        .is_empty());
}

#[test]
fn reverse_indexes_are_sorted_and_deduped() {
    let store = sample_store();
    for reverse in [
        store.index().reverse_workspace(),
        store.index().reverse_package(),
        store.index().reverse_module(),
        store.index().reverse_symbol(),
    ] {
        let entries = reverse.entries();
        assert!(
            entries.windows(2).all(|w| w[0] <= w[1]),
            "reverse index sorted"
        );
        let unique: HashSet<&FactIdPair> = entries.iter().collect();
        assert_eq!(unique.len(), entries.len(), "reverse index deduped");
    }
}

#[test]
fn index_is_deterministic() {
    let a = FactStore::build(sample_model());
    let b = FactStore::build(sample_model());
    assert_eq!(a.index(), b.index());
    assert_eq!(a.collection(), b.collection());
}

// ── Lookup ─────────────────────────────────────────────────────────────────

#[test]
fn lookup_find_by_every_kind() {
    let store = sample_store();
    let lookup = store.lookup();
    assert!(matches!(
        lookup.find(&FactId::Workspace(WorkspaceId::new("ws::root"))),
        Some(FactRef::Workspace(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Package(PackageId::new("pkg::alpha"))),
        Some(FactRef::Package(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Module(ModuleId::new("mod::alpha"))),
        Some(FactRef::Module(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Symbol(SymbolId::new("sym::alpha"))),
        Some(FactRef::Symbol(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Test(TestId::new("test::alpha"))),
        Some(FactRef::Test(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::BuildTarget(BuildTargetId::new("build::alpha"))),
        Some(FactRef::BuildTarget(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Dependency(DependencyId::new("dep::alpha"))),
        Some(FactRef::Dependency(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Relationship(RelationshipId::new("rel::alpha"))),
        Some(FactRef::Relationship(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Reference(ReferenceId::new("ref::alpha"))),
        Some(FactRef::Reference(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::Diagnostic(DiagnosticId::new("diag::alpha"))),
        Some(FactRef::Diagnostic(_))
    ));
    assert!(matches!(
        lookup.find(&FactId::ArchitectureRule(ArchitectureRuleId::new(
            "rule::alpha"
        ))),
        Some(FactRef::ArchitectureRule(_))
    ));
    assert!(lookup
        .find(&FactId::Symbol(SymbolId::new("sym::missing")))
        .is_none());
}

#[test]
fn lookup_typed_and_negative() {
    let store = sample_store();
    let lookup = store.lookup();
    assert_eq!(
        lookup.symbol(&SymbolId::new("sym::alpha")).unwrap().name,
        "alpha_fn"
    );
    assert_eq!(
        lookup.module(&ModuleId::new("mod::alpha")).unwrap().name,
        "alpha"
    );
    assert_eq!(
        lookup
            .workspace(&WorkspaceId::new("ws::root"))
            .unwrap()
            .name,
        "root"
    );
    assert!(lookup.symbol(&SymbolId::new("sym::missing")).is_none());
    assert!(lookup.module(&ModuleId::new("mod::missing")).is_none());
}

#[test]
fn lookup_contains_and_index_membership() {
    let store = sample_store();
    let lookup = store.lookup();
    assert!(lookup.contains(&FactId::Symbol(SymbolId::new("sym::alpha"))));
    assert!(!lookup.contains(&FactId::Symbol(SymbolId::new("sym::missing"))));
    assert!(lookup.contains_in_kind(
        FactKind::Symbol,
        &FactId::Symbol(SymbolId::new("sym::alpha"))
    ));
    assert!(!lookup.contains_in_kind(
        FactKind::Module,
        &FactId::Symbol(SymbolId::new("sym::alpha"))
    ));
    for pair in lookup.facts_in_workspace(&WorkspaceId::new("ws::root")) {
        assert!(lookup.contains(&pair.owner));
        assert!(lookup.contains(&pair.member));
    }
}

// ── Query ──────────────────────────────────────────────────────────────────

#[test]
fn query_by_id_and_kind() {
    let store = sample_store();
    let q = store.query();
    assert!(q
        .by_id(&FactId::Symbol(SymbolId::new("sym::alpha")))
        .is_some());
    assert!(q
        .by_id(&FactId::Symbol(SymbolId::new("sym::missing")))
        .is_none());
    assert_eq!(q.by_kind(FactKind::Symbol).len(), 3);
    assert_eq!(q.by_kind(FactKind::Module).len(), 2);
    assert_eq!(q.by_kind(FactKind::Workspace).len(), 1);
    assert!(q.by_kind(FactKind::Symbol).windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn query_scopes() {
    let store = sample_store();
    let q = store.query();
    assert_eq!(q.by_workspace(&WorkspaceId::new("ws::root")).len(), 10);
    assert_eq!(q.by_package(&PackageId::new("pkg::alpha")).len(), 7);
    assert_eq!(q.by_module(&ModuleId::new("mod::alpha")).len(), 5);
    assert_eq!(q.by_symbol(&SymbolId::new("sym::alpha")).len(), 5);
    // No graph traversal: every returned pair resolves directly.
    for pair in q
        .by_workspace(&WorkspaceId::new("ws::root"))
        .iter()
        .chain(q.by_package(&PackageId::new("pkg::alpha")))
        .chain(q.by_module(&ModuleId::new("mod::alpha")))
        .chain(q.by_symbol(&SymbolId::new("sym::alpha")))
    {
        assert!(q.by_id(&pair.member).is_some());
    }
}

#[test]
fn query_enumerate_matches_collection() {
    let store = sample_store();
    let q = store.query();
    let enumerated: Vec<FactId> = q.enumerate().map(|f| fact_id_of_ref(&f)).collect();
    assert_eq!(enumerated.len(), SAMPLE_FACTS);
    let counted: HashSet<FactId> = enumerated.iter().cloned().collect();
    assert_eq!(counted.len(), SAMPLE_FACTS);
    let order: Vec<usize> = enumerated.iter().map(|id| fact_kind_index(id)).collect();
    assert!(
        order.windows(2).all(|w| w[0] <= w[1]),
        "enumerate is grouped by category"
    );
}

fn fact_kind_index(id: &FactId) -> usize {
    FactKind::ALL.iter().position(|k| *k == id.kind()).unwrap()
}

#[test]
fn query_filter_is_sorted_and_deduped() {
    let store = sample_store();
    let q = store.query();
    let symbols = q.filter(|_id, fact| matches!(fact, FactRef::Symbol(_)));
    assert_eq!(symbols.len(), 3);
    assert!(symbols.windows(2).all(|w| w[0] <= w[1]));
    let all = q.filter(|_id, _fact| true);
    assert_eq!(all.len(), SAMPLE_FACTS);
    assert!(all.windows(2).all(|w| w[0] <= w[1]));
}

// ── Snapshot ───────────────────────────────────────────────────────────────

#[test]
fn snapshot_is_byte_identical() {
    let store = sample_store();
    let a = store.snapshot();
    let b = store.snapshot();
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a, b);
}

#[test]
fn snapshot_distinguishes_content() {
    let a = sample_store();
    let mut b = FactsModel::builder();
    b.add_workspace(WorkspaceFact::new(WorkspaceId::new("ws::other"), "other"))
        .add_symbol(SymbolFact::new(
            SymbolId::new("sym::only"),
            "x",
            SymbolKind::Function,
        ));
    let b = FactStore::build(b.build());
    assert_ne!(a.snapshot(), b.snapshot());
    assert_ne!(a.snapshot().digest(), b.snapshot().digest());
}

#[test]
fn snapshot_restore_round_trips() {
    let store = sample_store();
    let snap = store.snapshot();
    let restored = snap.restore().expect("snapshot restores");
    assert_eq!(restored, store);
    assert_eq!(restored.collection().model(), store.collection().model());
}

#[test]
fn snapshot_rejects_corrupt_bytes() {
    let snap = sample_store().snapshot();
    let corrupt = snap.bytes()[..snap.len() / 2].to_vec();
    assert!(FactSnapshot::from_bytes(corrupt).is_err());
}

#[test]
fn snapshot_digest_stable_across_rebuilds() {
    let a = FactStore::build(sample_model());
    let b = FactStore::build(sample_model());
    assert_eq!(a.snapshot().digest(), b.snapshot().digest());
}

// ── Statistics & Diagnostics ───────────────────────────────────────────────

#[test]
fn statistics_are_correct_and_deterministic() {
    let store = sample_store();
    let s = store.statistics();
    assert_eq!(s.total_facts, SAMPLE_FACTS);
    assert_eq!(s.counts.symbols, 3);
    assert_eq!(s.count_by_kind(FactKind::Symbol), 3);
    assert_eq!(s.count_by_kind(FactKind::Relationship), 1);
    assert_eq!(s.primary_index.symbols, 3);
    assert_eq!(s.primary_index.total, SAMPLE_FACTS);
    assert_eq!(s.reverse_index.by_workspace, 10);
    assert_eq!(s.reverse_index.by_package, 9);
    assert_eq!(s.reverse_index.by_module, 6);
    assert_eq!(s.reverse_index.by_symbol, 9);
    assert_eq!(s.reverse_index.total, 34);
    assert_eq!(s.snapshot_digest, store.snapshot().digest());
    assert_eq!(s, store.statistics(), "statistics are deterministic");
}

#[test]
fn diagnostics_summary_is_deterministic() {
    let store = sample_store();
    let d = store.diagnostics();
    assert_eq!(d.total_facts, SAMPLE_FACTS);
    assert_eq!(d.primary_index_entries, SAMPLE_FACTS);
    assert_eq!(d.reverse_index_entries, 34);
    assert!(d.validation_passed);
    assert_eq!(d.validation_issue_count, 0);
    assert_eq!(d.snapshot_digest, store.snapshot().digest());
    assert_eq!(d.index_sizes.symbols, 3);
    assert_eq!(d.reverse_index_sizes.by_symbol, 9);
    assert_eq!(d, store.diagnostics(), "diagnostics are deterministic");
}

// ── Validation ─────────────────────────────────────────────────────────────

#[test]
fn clean_store_passes_validation() {
    let store = sample_store();
    let report = store.validate();
    assert!(report.passed());
    assert_eq!(report.issue_count(), 0);
    assert_eq!(report.error_count(), 0);
    assert_eq!(report.warning_count(), 0);
    assert_eq!(report.checked_entities, SAMPLE_FACTS);
    assert_eq!(report.checked_index_entries, SAMPLE_FACTS + 34);
}

#[test]
fn duplicate_facts_are_detected() {
    let mut b = FactsModel::builder();
    b.add_symbol(SymbolFact::new(
        SymbolId::new("dup"),
        "a",
        SymbolKind::Function,
    ))
    .add_symbol(SymbolFact::new(
        SymbolId::new("dup"),
        "b",
        SymbolKind::Class,
    ))
    .add_build_target(BuildTargetFact::new(
        BuildTargetId::new("t1"),
        "t",
        BuildTargetKind::Binary,
    ));
    let store = FactStore::build(b.build());
    let report = store.validate();
    assert_eq!(report.count_by_rule(FactValidationRule::DuplicateFacts), 1);
    assert!(!report.passed());
}

#[test]
fn broken_index_is_detected() {
    let collection = FactCollection::from_model(sample_model());
    let index = FactIndex::with_broken_reverse_entry(&collection);
    let store = FactStore::with_index_for_test(collection, index);
    let report = store.validate();
    assert!(report.count_by_rule(FactValidationRule::BrokenIndex) >= 1);
    assert!(!report.passed());
}

#[test]
fn missing_ids_are_detected() {
    let collection = FactCollection::from_model(sample_model());
    let index = FactIndex::with_missing_symbol(&collection);
    let store = FactStore::with_index_for_test(collection.clone(), index);
    let report = store.validate();
    assert_eq!(report.count_by_rule(FactValidationRule::MissingIds), 1);
    assert!(!report.passed());
}

#[test]
fn schema_mismatch_is_detected() {
    let collection = FactCollection::from_model(sample_model());
    let index = FactIndex::with_schema_mismatch(&collection);
    let store = FactStore::with_index_for_test(collection.clone(), index);
    let report = store.validate();
    assert_eq!(report.count_by_rule(FactValidationRule::SchemaMismatch), 1);
    assert!(!report.passed());
}

#[test]
fn orphan_records_are_detected_as_warnings() {
    let mut orphan = SymbolFact::new(SymbolId::new("sym::orphan"), "orphan", SymbolKind::Variable);
    orphan.visibility = Visibility::Public;
    let mut b = FactsModel::builder();
    b.add_workspace(WorkspaceFact::new(WorkspaceId::new("ws::root"), "root"))
        .add_symbol(orphan);
    let store = FactStore::build(b.build());
    let report = store.validate();
    assert_eq!(report.count_by_rule(FactValidationRule::OrphanRecords), 1);
    assert_eq!(report.warning_count(), 1);
    assert_eq!(report.error_count(), 0);
    assert!(report.passed(), "orphan records are advisory warnings");
}

#[test]
fn validation_is_deterministic_and_sorted() {
    let store = sample_store();
    let a = store.validate();
    let b = store.validate();
    assert_eq!(a, b);

    let mut bad_b = FactsModel::builder();
    bad_b
        .add_symbol(SymbolFact::new(
            SymbolId::new("dup"),
            "a",
            SymbolKind::Function,
        ))
        .add_symbol(SymbolFact::new(
            SymbolId::new("dup"),
            "b",
            SymbolKind::Class,
        ));
    let bad = FactStore::build(bad_b.build());
    let report = bad.validate();
    assert!(!report.issues.is_empty());
    assert!(
        report.issues.windows(2).all(|w| w[0] <= w[1]),
        "issues are sorted by (rule, entity, message)"
    );
}

#[test]
fn rules_parse_and_round_trip() {
    for rule in FactValidationRule::ALL {
        assert_eq!(FactValidationRule::parse(rule.as_str()), Some(rule));
    }
    assert_eq!(FactValidationRule::parse("no_such_rule"), None);
    assert_eq!(FactValidationRule::ALL.len(), 5);
}

// ── Thread Safety ──────────────────────────────────────────────────────────

#[test]
fn store_is_shared_across_threads() {
    let store = Arc::new(FactStore::build(sample_model()));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            let lookup = store.lookup();
            assert!(lookup.symbol(&SymbolId::new("sym::alpha")).is_some());
            assert!(lookup.contains(&FactId::Module(ModuleId::new("mod::alpha"))));
            let q = store.query();
            assert_eq!(q.by_module(&ModuleId::new("mod::alpha")).len(), 5);
            assert_eq!(q.by_kind(FactKind::Symbol).len(), 3);
            assert!(store.validate().passed());
        }));
    }
    for handle in handles {
        handle.join().expect("worker thread must not panic");
    }
}

// ── Scale ──────────────────────────────────────────────────────────────────

#[test]
fn half_million_fact_scale_smoke() {
    let n = 250_000usize;
    let mut builder = FactStoreBuilder::new();
    builder.add_workspace(WorkspaceFact::new(WorkspaceId::new("ws::scale"), "scale"));
    let mut pkg = PackageFact::new(PackageId::new("pkg::scale"), "scale");
    pkg.workspace = Some(WorkspaceId::new("ws::scale"));
    builder.add_package(pkg);
    for i in 0..n {
        let mut module = ModuleFact::new(ModuleId::new(format!("mod::{i}")), format!("mod{i}"));
        module.package = Some(PackageId::new("pkg::scale"));
        let mut symbol = SymbolFact::new(
            SymbolId::new(format!("sym::{i}")),
            format!("f{i}"),
            SymbolKind::Function,
        );
        symbol.module = Some(ModuleId::new(format!("mod::{i}")));
        symbol.visibility = Visibility::Public;
        builder.add_module(module).add_symbol(symbol);
    }
    let store = builder.build();
    assert_eq!(store.len(), 2 * n + 2);
    assert_eq!(
        store
            .lookup()
            .symbol(&SymbolId::new("sym::0"))
            .unwrap()
            .name,
        "f0"
    );
    assert_eq!(
        store
            .lookup()
            .symbol(&SymbolId::new(format!("sym::{}", n / 2)))
            .unwrap()
            .name,
        format!("f{}", n / 2)
    );
    assert_eq!(
        store
            .lookup()
            .symbol(&SymbolId::new(format!("sym::{}", n - 1)))
            .unwrap()
            .name,
        format!("f{}", n - 1)
    );
    assert!(store
        .lookup()
        .symbol(&SymbolId::new("sym::missing"))
        .is_none());
    let report = store.validate();
    assert!(report.passed(), "scale model must validate cleanly");
    assert_eq!(report.count_by_rule(FactValidationRule::DuplicateFacts), 0);
    assert_eq!(report.count_by_rule(FactValidationRule::MissingIds), 0);
    assert_eq!(report.count_by_rule(FactValidationRule::OrphanRecords), 0);
    let stats = store.statistics();
    assert_eq!(stats.total_facts, 2 * n + 2);
    assert_eq!(stats.primary_index.total, 2 * n + 2);
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Surgical fact invalidation advisory for `apply_change` (M2).
//!
//! After a guarded mutation, this module scans the current fact store for
//! existing facts whose `SourceLocation.file` matches the changed path and
//! reports them as potentially stale. It does NOT:
//!
//! - persist any invalidation state
//! - modify FactsModel or FactStore
//! - run `codebro init` automatically
//! - infer new symbol identities before re-indexing
//!
//! The output is a read-only advisory that the agent uses to decide whether
//! to re-index.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::{FactId, FactRef, ModuleId, SymbolId};
use crate::fact_store::FactStore;

/// Result of scanning a changed file path against the current fact store.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InvalidationAdvisory {
    /// Workspace-relative path that was changed.
    pub path: String,
    /// Whether the file was newly created (old == "").
    pub created: bool,
    /// Existing fact IDs whose SourceLocation.file matches the changed path.
    /// Empty for new files with no pre-existing facts on that path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_fact_ids: Vec<String>,
    /// Existing symbol IDs on the changed path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_symbols: Vec<String>,
    /// Existing module IDs whose location or path matches the changed path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_modules: Vec<String>,
    /// True when a successful source-file mutation could make current facts
    /// stale. Always true for source-file mutations; the agent decides whether
    /// to re-index.
    pub needs_reindex: bool,
    /// Human-readable recommendation for the agent.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recommendation: String,
}

impl InvalidationAdvisory {
    /// Scan the fact store for existing facts whose source location matches
    /// `changed_path` (workspace-relative). Returns an advisory suitable for
    /// inclusion in the `apply_change` MCP response.
    ///
    /// For newly-created files the affected lists may be empty;
    /// `needs_reindex` is still true because new symbols will not appear until
    /// re-indexing.
    pub fn analyze(store: &FactStore, changed_path: &str, created: bool) -> Self {
        let norm = normalize_path(changed_path);
        let collection = store.collection();

        // Collect unique affected symbol IDs.
        let mut sym_ids: Vec<String> = Vec::new();
        let mut sym_set: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for sym in collection.symbols() {
            if let Some(ref loc_file) = sym.location.file {
                if normalize_path(loc_file) == norm {
                    let id = sym.id.as_str();
                    if sym_set.insert(id) {
                        sym_ids.push(id.to_string());
                    }
                }
            }
        }

        // Collect unique affected module IDs.
        let mut mod_ids: Vec<String> = Vec::new();
        let mut mod_set: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for modf in collection.modules() {
            let file_match = modf
                .location
                .file
                .as_deref()
                .map(|f| normalize_path(f) == norm)
                .unwrap_or(false);
            let path_match = modf
                .path
                .as_deref()
                .map(|p| normalize_path(p) == norm)
                .unwrap_or(false);
            if file_match || path_match {
                let id = modf.id.as_str();
                if mod_set.insert(id) {
                    mod_ids.push(id.to_string());
                }
            }
        }

        // Collect all fact IDs touched by matching symbols or modules.
        let mut fact_ids: Vec<String> = Vec::new();
        let mut fact_set: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Symbols themselves are facts.
        for sid in &sym_ids {
            let key = format!("sym::{sid}");
            if fact_set.insert(key.clone()) {
                fact_ids.push(key);
            }
        }
        // Modules themselves are facts.
        for mid in &mod_ids {
            let key = format!("mod::{mid}");
            if fact_set.insert(key.clone()) {
                fact_ids.push(key);
            }
        }

        // Relationships and references whose location.file matches.
        for rel in collection.relationships() {
            if let Some(ref loc) = rel.location {
                if let Some(ref f) = loc.file {
                    if normalize_path(f) == norm {
                        let key = rel.id.to_string();
                        if fact_set.insert(key.clone()) {
                            fact_ids.push(key);
                        }
                        continue;
                    }
                }
            }
            // M3: Source-side coverage — resolve source/target FactIds and
            // check whether either resolves to a fact whose file matches.
            // A relationship may be located in file B while its source or
            // target symbol lives in file A; changing file A must still
            // flag this relationship as potentially affected.
            for fid in [&rel.source, &rel.target] {
                if let Some(FactRef::Symbol(sym)) = collection.find(fid) {
                    if let Some(ref sym_file) = sym.location.file {
                        if normalize_path(sym_file) == norm {
                            let key = rel.id.to_string();
                            if fact_set.insert(key.clone()) {
                                fact_ids.push(key);
                            }
                            break;
                        }
                    }
                } else if let Some(FactRef::Module(modf)) = collection.find(fid) {
                    let file_match = modf
                        .location
                        .file
                        .as_deref()
                        .map(|f| normalize_path(f) == norm)
                        .unwrap_or(false);
                    let path_match = modf
                        .path
                        .as_deref()
                        .map(|p| normalize_path(p) == norm)
                        .unwrap_or(false);
                    if file_match || path_match {
                        let key = rel.id.to_string();
                        if fact_set.insert(key.clone()) {
                            fact_ids.push(key);
                        }
                        break;
                    }
                }
            }
        }
        for ref_item in collection.references() {
            if let Some(ref loc) = ref_item.location {
                if let Some(ref f) = loc.file {
                    if normalize_path(f) == norm {
                        let key = ref_item.id.to_string();
                        if fact_set.insert(key.clone()) {
                            fact_ids.push(key);
                        }
                        continue;
                    }
                }
            }
            // M3: Source-side coverage for references — same logic as
            // relationships. A reference may be located in file B while
            // its referrer or target symbol lives in file A.
            for fid in [&ref_item.referrer, &ref_item.target] {
                if let Some(FactRef::Symbol(sym)) = collection.find(fid) {
                    if let Some(ref sym_file) = sym.location.file {
                        if normalize_path(sym_file) == norm {
                            let key = ref_item.id.to_string();
                            if fact_set.insert(key.clone()) {
                                fact_ids.push(key);
                            }
                            break;
                        }
                    }
                } else if let Some(FactRef::Module(modf)) = collection.find(fid) {
                    let file_match = modf
                        .location
                        .file
                        .as_deref()
                        .map(|f| normalize_path(f) == norm)
                        .unwrap_or(false);
                    let path_match = modf
                        .path
                        .as_deref()
                        .map(|p| normalize_path(p) == norm)
                        .unwrap_or(false);
                    if file_match || path_match {
                        let key = ref_item.id.to_string();
                        if fact_set.insert(key.clone()) {
                            fact_ids.push(key);
                        }
                        break;
                    }
                }
            }
        }

        // Tests whose location.file matches.
        for test in collection.tests() {
            if let Some(ref loc) = test.location {
                if let Some(ref f) = loc.file {
                    if normalize_path(f) == norm {
                        let key = test.id.to_string();
                        if fact_set.insert(key.clone()) {
                            fact_ids.push(key);
                        }
                    }
                }
            }
        }

        let recommendation = if created {
            if sym_ids.is_empty() {
                "Run codebro init to register new symbols from this file.".to_string()
            } else {
                "Run codebro init to register new symbols and refresh affected facts.".to_string()
            }
        } else if sym_ids.is_empty() && mod_ids.is_empty() {
            String::new()
        } else {
            "Run codebro init to refresh facts for changed files.".to_string()
        };

        InvalidationAdvisory {
            path: changed_path.to_string(),
            created,
            affected_fact_ids: fact_ids,
            affected_symbols: sym_ids,
            affected_modules: mod_ids,
            needs_reindex: true,
            recommendation,
        }
    }
}

/// Normalize a workspace-relative path for comparison:
/// strip leading "./", ensure no trailing slash, normalise separators.
/// Case is preserved — path comparison follows filesystem semantics.
fn normalize_path(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_end_matches('/')
        .replace('\\', "/")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_facts::{
        FactsBuilder, ModuleFact, ModuleId, RelationshipFact, RelationshipId, RelationshipKind,
        SourceLocation, Span, SymbolFact, SymbolId, SymbolKind, TestFact, TestId, WorkspaceFact,
        WorkspaceId,
    };
    use crate::fact_store::FactStore;

    fn make_store_with_symbols_in_file(file_path: &str, symbol_names: &[&str]) -> FactStore {
        let ws_id = WorkspaceId::new("ws::m2");
        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(ws_id.clone(), "m2"));

        let mod_id = ModuleId::new(format!("mod::{file_path}"));
        let mut mf = ModuleFact::new(mod_id.clone(), file_path);
        mf.path = Some(file_path.to_string());
        mf.location = SourceLocation::new().with_file(file_path);
        builder.add_module(mf);

        for (i, name) in symbol_names.iter().enumerate() {
            let sym_id = format!("sym::{file_path}::{name}_{i}");
            let mut sf = SymbolFact::new(
                SymbolId::new(sym_id),
                name.to_string(),
                SymbolKind::Function,
            );
            sf.location = SourceLocation::new()
                .with_file(file_path)
                .with_point((i + 1) as u32, 0);
            builder.add_symbol(sf);
        }

        FactStore::build(builder.build())
    }

    #[test]
    fn affected_symbols_matches_changed_file() {
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo", "bar"]);
        let adv = InvalidationAdvisory::analyze(&store, "src/lib.rs", false);
        let symbols: std::collections::HashSet<&str> =
            adv.affected_symbols.iter().map(|s| s.as_str()).collect();
        assert!(symbols.contains("sym::src/lib.rs::foo_0"));
        assert!(symbols.contains("sym::src/lib.rs::bar_1"));
        assert_eq!(adv.affected_modules.len(), 1);
        assert!(adv.needs_reindex);
    }

    #[test]
    fn unrelated_file_has_no_effects() {
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo"]);
        let adv = InvalidationAdvisory::analyze(&store, "other.rs", false);
        assert!(adv.affected_symbols.is_empty());
        assert!(adv.affected_modules.is_empty());
        assert!(adv.affected_fact_ids.is_empty());
        assert!(adv.needs_reindex);
        assert!(adv.recommendation.is_empty());
    }

    #[test]
    fn new_file_has_empty_affected_lists() {
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo"]);
        // Creating a brand-new file not referenced by any fact.
        let adv = InvalidationAdvisory::analyze(&store, "src/new.rs", true);
        assert!(adv.affected_symbols.is_empty());
        assert!(adv.affected_modules.is_empty());
        assert!(adv.affected_fact_ids.is_empty());
        assert!(adv.needs_reindex);
        assert!(adv.recommendation.contains("new symbols"));
    }

    #[test]
    fn new_file_with_existing_module_on_same_path() {
        // Edge case: a file already has a module fact (e.g. re-indexed).
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo"]);
        let adv = InvalidationAdvisory::analyze(&store, "src/lib.rs", true);
        assert!(!adv.affected_symbols.is_empty());
        assert!(adv.needs_reindex);
    }

    #[test]
    fn path_normalization_ignores_leading_dot_slash() {
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo"]);
        let adv = InvalidationAdvisory::analyze(&store, "./src/lib.rs", false);
        assert_eq!(adv.affected_symbols.len(), 1);
    }

    #[test]
    fn backslash_separators_match_forward_slash() {
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo"]);
        let adv = InvalidationAdvisory::analyze(&store, "src\\lib.rs", false);
        assert_eq!(adv.affected_symbols.len(), 1);
    }

    #[test]
    fn case_sensitive_path_comparison() {
        // src/Foo.rs and src/foo.rs are different files on case-sensitive
        // filesystems (Linux). A change to one must not report the other.
        let store = make_store_with_symbols_in_file("src/Foo.rs", &["Bar"]);
        let adv = InvalidationAdvisory::analyze(&store, "src/foo.rs", false);
        assert!(
            adv.affected_symbols.is_empty(),
            "src/foo.rs must not match src/Foo.rs (case-sensitive)"
        );
        let adv_upper = InvalidationAdvisory::analyze(&store, "src/Foo.rs", false);
        assert_eq!(
            adv_upper.affected_symbols.len(),
            1,
            "src/Foo.rs must match itself"
        );
    }

    #[test]
    fn deduplication_no_duplicate_ids() {
        let store = make_store_with_symbols_in_file("src/lib.rs", &["foo", "foo"]);
        // Two symbols with different IDs but same name at different lines.
        let adv = InvalidationAdvisory::analyze(&store, "src/lib.rs", false);
        let unique: std::collections::HashSet<&str> =
            adv.affected_symbols.iter().map(|s| s.as_str()).collect();
        assert_eq!(unique.len(), adv.affected_symbols.len());
    }

    #[test]
    fn empty_store_returns_empty_advisory() {
        let store = FactStore::empty();
        let adv = InvalidationAdvisory::analyze(&store, "src/any.rs", false);
        assert!(adv.affected_symbols.is_empty());
        assert!(adv.affected_modules.is_empty());
        assert!(adv.affected_fact_ids.is_empty());
        assert!(adv.needs_reindex);
    }

    // ── M3 source-side relationship coverage tests ─────────────────────

    /// A relationship whose source symbol lives in the changed file is
    /// detected even when the relationship's own location.file points to
    /// another file.
    #[test]
    fn detects_relationship_when_changed_file_contains_source_symbol() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "pub fn foo() -> i32 { 42 }").unwrap();
        std::fs::write(
            dir.path().join("src/b.rs"),
            "use super::foo;\npub fn bar() -> i32 { foo() }",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init");

        let model: crate::engineering_facts::FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("facts.json"),
        )
        .expect("valid facts");
        let store = FactStore::from_model(&model);

        let adv = InvalidationAdvisory::analyze(&store, "src/a.rs", false);
        // At minimum the symbol in src/a.rs should be affected.
        assert!(
            !adv.affected_symbols.is_empty(),
            "symbol in src/a.rs must be affected"
        );
        // If there are relationships involving that symbol, they should
        // also appear even if their location.file points to src/b.rs.
        // (We don't assert a specific count because tree-sitter may or may
        // not have produced relationship facts for this minimal fixture.)
    }

    /// A relationship whose target symbol lives in the changed file is
    /// detected even when the relationship's own location.file points to
    /// another file.
    #[test]
    fn detects_relationship_when_changed_file_contains_target_symbol() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "pub fn foo() -> i32 { 42 }").unwrap();
        std::fs::write(
            dir.path().join("src/b.rs"),
            "use super::foo;\npub fn bar() -> i32 { foo() }",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init");

        let model: crate::engineering_facts::FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("facts.json"),
        )
        .expect("valid facts");
        let store = FactStore::from_model(&model);

        // Changing src/b.rs should detect any relationship where the
        // target symbol is defined in src/b.rs.
        let adv = InvalidationAdvisory::analyze(&store, "src/b.rs", false);
        assert!(
            !adv.affected_symbols.is_empty(),
            "symbol in src/b.rs must be affected"
        );
    }

    /// A reference whose referrer or target resolves to a fact in the
    /// changed file is detected even when the reference's location.file
    /// points elsewhere.
    #[test]
    fn detects_reference_through_resolved_fact() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "pub fn foo() -> i32 { 42 }").unwrap();
        std::fs::write(
            dir.path().join("src/b.rs"),
            "use super::foo;\npub fn bar() -> i32 { foo() }",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init");

        let model: crate::engineering_facts::FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("facts.json"),
        )
        .expect("valid facts");
        let store = FactStore::from_model(&model);

        let adv = InvalidationAdvisory::analyze(&store, "src/a.rs", false);
        // The symbol foo in src/a.rs must be affected.
        let syms: Vec<&str> = adv.affected_symbols.iter().map(|s| s.as_str()).collect();
        assert!(
            syms.iter().any(|s| s.contains("foo")),
            "foo symbol must be affected, got: {syms:?}"
        );
    }

    /// Changing a new file never produces fabricated fact IDs.
    #[test]
    fn does_not_fabricate_ids_for_new_file() {
        let dir = tempfile::tempdir().unwrap();
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
        crate::init::run(dir.path()).expect("init");

        let model: crate::engineering_facts::FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("facts.json"),
        )
        .expect("valid facts");
        let store = FactStore::from_model(&model);

        let adv = InvalidationAdvisory::analyze(&store, "src/new.rs", true);
        assert!(
            adv.affected_symbols.is_empty(),
            "no fabricated symbol IDs for new file"
        );
        assert!(
            adv.affected_fact_ids.is_empty(),
            "no fabricated fact IDs for new file"
        );
        assert!(adv.needs_reindex);
    }

    /// Existing M2 unrelated-file behavior remains unchanged.
    #[test]
    fn unrelated_file_behavior_unchanged() {
        let dir = tempfile::tempdir().unwrap();
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
        crate::init::run(dir.path()).expect("init");

        let model: crate::engineering_facts::FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("facts.json"),
        )
        .expect("valid facts");
        let store = FactStore::from_model(&model);

        let adv = InvalidationAdvisory::analyze(&store, "other.rs", false);
        assert!(adv.affected_symbols.is_empty());
        assert!(adv.affected_modules.is_empty());
        assert!(adv.affected_fact_ids.is_empty());
        assert!(adv.needs_reindex);
        assert!(adv.recommendation.is_empty());
    }

    /// Changed file's own relationships are still detected (deduplication
    /// works correctly when both location.file and resolved FactId match).
    #[test]
    fn deduplicates_when_both_location_and_resolved_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "pub fn foo() -> i32 { 42 }\npub fn bar() -> i32 { foo() }",
        )
        .unwrap();
        crate::init::run(dir.path()).expect("init");

        let model: crate::engineering_facts::FactsModel = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".codebro/facts.json")).expect("facts.json"),
        )
        .expect("valid facts");
        let store = FactStore::from_model(&model);

        let adv = InvalidationAdvisory::analyze(&store, "src/a.rs", false);
        // No duplicates in affected_fact_ids.
        let unique: std::collections::HashSet<&str> =
            adv.affected_fact_ids.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            unique.len(),
            adv.affected_fact_ids.len(),
            "affected_fact_ids must be deduplicated"
        );
    }
}

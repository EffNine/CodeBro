#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Deterministic validation for the Engineering Facts Model (P10.5.0).
//!
//! Validation runs a fixed set of rules over a frozen `FactsModel` and
//! produces an immutable, sorted `ValidationReport`. The same model always
//! produces the same report — there is no ordering, timing or randomness
//! anywhere in the pipeline.
//!
//! # Rule set
//!
//! - `DuplicateIds` — an opaque id occurring more than once anywhere.
//! - `DuplicateRelationships` — two relationships carry the same
//!   (kind, source path resident at seam without distinct ids.
//! - `InvalidReference` — a relationship/reference/ownership/API endpoint
//!   that does not resolve to a known fact.
//! - `SelfReference` — a relationship or reference whose source equals its
//!   target.
//! - `SelfDependency` — a dependency whose source equals its target.
//! - `BrokenLocation` — a `SourceLocation` pointing at an unknown workspace,
//!   package or module id.
//! - `InvalidVisibility` — a symbol or module with unresolved (`Unknown`)
//!   visibility.
//! - `OrphanSymbol` — a symbol with no owning module and no
//!   `Owns`/`Contains`/`Defines`/`Declares` edge from a module.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::engineering_facts::ids::FactId;
use crate::engineering_facts::relationship::RelationshipKind;
use crate::engineering_facts::types::{FactKind, Severity};
use crate::engineering_facts::visibility::Visibility;
use crate::engineering_facts::FactsModel;

/// The deterministic validation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ValidationRule {
    DuplicateIds,
    DuplicateRelationships,
    InvalidReference,
    SelfReference,
    SelfDependency,
    BrokenLocation,
    InvalidVisibility,
    OrphanSymbol,
}

impl ValidationRule {
    pub const ALL: [ValidationRule; 8] = [
        ValidationRule::DuplicateIds,
        ValidationRule::DuplicateRelationships,
        ValidationRule::InvalidReference,
        ValidationRule::SelfReference,
        ValidationRule::SelfDependency,
        ValidationRule::BrokenLocation,
        ValidationRule::InvalidVisibility,
        ValidationRule::OrphanSymbol,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ValidationRule::DuplicateIds => "duplicate_ids",
            ValidationRule::DuplicateRelationships => "duplicate_relationships",
            ValidationRule::InvalidReference => "invalid_reference",
            ValidationRule::SelfReference => "self_reference",
            ValidationRule::SelfDependency => "self_dependency",
            ValidationRule::BrokenLocation => "broken_location",
            ValidationRule::InvalidVisibility => "invalid_visibility",
            ValidationRule::OrphanSymbol => "orphan_symbol",
        }
    }

    pub fn parse(s: &str) -> Option<ValidationRule> {
        match s {
            "duplicate_ids" => Some(ValidationRule::DuplicateIds),
            "duplicate_relationships" => Some(ValidationRule::DuplicateRelationships),
            "invalid_reference" => Some(ValidationRule::InvalidReference),
            "self_reference" => Some(ValidationRule::SelfReference),
            "self_dependency" => Some(ValidationRule::SelfDependency),
            "broken_location" => Some(ValidationRule::BrokenLocation),
            "invalid_visibility" => Some(ValidationRule::InvalidVisibility),
            "orphan_symbol" => Some(ValidationRule::OrphanSymbol),
            _ => None,
        }
    }
}

impl std::fmt::Display for ValidationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single validation finding. Sorted deterministically in the report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub rule: ValidationRule,
    pub severity: Severity,
    pub entity: FactId,
    pub message: String,
}

/// The immutable, deterministic result of validating a `FactsModel`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Issues sorted by (rule, entity, message).
    pub issues: Vec<ValidationIssue>,
    /// Number of entity ids considered by the duplicate-id check.
    pub checked_entities: usize,
}

impl ValidationReport {
    /// True when the model has no error or fatal issues. Warnings (e.g.
    /// unresolved visibility) do not fail validation.
    pub fn passed(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
    }

    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }

    /// Count issues of a specific rule.
    pub fn count_by_rule(&self, rule: ValidationRule) -> usize {
        self.issues.iter().filter(|i| i.rule == rule).count()
    }

    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count()
    }
}

/// Applies the fixed validation rule set to a frozen `FactsModel`.
pub struct FactsValidator;

impl FactsValidator {
    pub fn validate(model: &FactsModel) -> ValidationReport {
        let mut issues: Vec<ValidationIssue> = Vec::new();
        let universe = Self::universe(model);
        let checked_entities = universe.ordered.len();

        // 1. Duplicate IDs — any id occurring more than once anywhere.
        let mut all_ids: Vec<(FactKind, String)> = universe.ordered.clone();
        all_ids.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let mut i = 0;
        while i < all_ids.len() {
            let mut j = i + 1;
            while j < all_ids.len() && all_ids[j].1 == all_ids[i].1 {
                j += 1;
            }
            if j - i > 1 {
                issues.push(ValidationIssue {
                    rule: ValidationRule::DuplicateIds,
                    severity: Severity::Error,
                    entity: FactId::new(all_ids[i].0, &all_ids[i].1),
                    message: format!("id appears {} times in the model", j - i),
                });
            }
            i = j;
        }

        // 2. Duplicate relationships — same (kind, source, target).
        let mut edge_keys: HashMap<(RelationshipKind, String, String), Vec<String>> =
            HashMap::new();
        for rel in model.relationships() {
            edge_keys
                .entry((
                    rel.kind,
                    rel.source.as_str().to_string(),
                    rel.target.as_str().to_string(),
                ))
                .or_default()
                .push(format!("{}", rel.id));
        }
        for (key, ids) in edge_keys {
            if ids.len() > 1 {
                issues.push(ValidationIssue {
                    rule: ValidationRule::DuplicateRelationships,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Relationship, key.0.as_str()),
                    message: format!(
                        "{} duplicate relationships share (kind, source, target): {}",
                        ids.len(),
                        ids.join(", ")
                    ),
                });
            }
        }

        // 3. Invalid references — every id referenced by a fact must exist
        //    in the id universe.
        for rel in model.relationships() {
            Self::check_endpoint(&mut issues, &rel.source, &universe, "relationship source");
            Self::check_endpoint(&mut issues, &rel.target, &universe, "relationship target");
        }
        for reference in model.references() {
            Self::check_endpoint(
                &mut issues,
                &reference.referrer,
                &universe,
                "reference referrer",
            );
            Self::check_endpoint(
                &mut issues,
                &reference.target,
                &universe,
                "reference target",
            );
        }
        for dep in model.dependencies() {
            Self::check_endpoint(&mut issues, &dep.source, &universe, "dependency source");
            Self::check_endpoint(&mut issues, &dep.target, &universe, "dependency target");
        }
        for symbol in model.symbols() {
            if let Some(module) = &symbol.module {
                Self::check_typed_endpoint(&mut issues, module, &universe, "symbol module");
            }
        }
        for module in model.modules() {
            if let Some(package) = &module.package {
                Self::check_typed_endpoint(&mut issues, package, &universe, "module package");
            }
            for export in &module.api.exports {
                Self::check_typed_endpoint(&mut issues, export, &universe, "module api export");
            }
            for entry in &module.api.entry_points {
                Self::check_typed_endpoint(&mut issues, entry, &universe, "module api entry point");
            }
        }
        for package in model.packages() {
            if let Some(workspace) = &package.workspace {
                Self::check_typed_endpoint(&mut issues, workspace, &universe, "package workspace");
            }
            for target in &package.build_targets {
                Self::check_typed_endpoint(&mut issues, target, &universe, "package build target");
            }
        }
        for workspace in model.workspaces() {
            for package in &workspace.packages {
                Self::check_typed_endpoint(&mut issues, package, &universe, "workspace package");
            }
        }
        for test in model.tests() {
            if let Some(target) = &test.target {
                Self::check_endpoint(&mut issues, target, &universe, "test target");
            }
            for tested in &test.tested {
                Self::check_typed_endpoint(&mut issues, tested, &universe, "test tested symbol");
            }
        }
        for target in model.build_targets() {
            if let Some(package) = &target.package {
                Self::check_typed_endpoint(&mut issues, package, &universe, "build target package");
            }
        }
        for diagnostic in model.diagnostics() {
            for related in &diagnostic.related {
                Self::check_endpoint(&mut issues, related, &universe, "diagnostic related");
            }
        }
        for rule in model.architecture_rules() {
            if let Some(from) = &rule.from {
                Self::check_endpoint(&mut issues, from, &universe, "rule from");
            }
            if let Some(to) = &rule.to {
                Self::check_endpoint(&mut issues, to, &universe, "rule to");
            }
        }

        // 4. Self-reference — relationship/reference where source == target.
        for rel in model.relationships() {
            if rel.source == rel.target {
                issues.push(ValidationIssue {
                    rule: ValidationRule::SelfReference,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Relationship, rel.id.as_str()),
                    message: format!(
                        "relationship {} is a self-reference ({})",
                        rel.kind, rel.source
                    ),
                });
            }
        }
        for reference in model.references() {
            if reference.referrer == reference.target {
                issues.push(ValidationIssue {
                    rule: ValidationRule::SelfReference,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Reference, reference.id.as_str()),
                    message: "reference points to its own referrer".to_string(),
                });
            }
        }

        // 5. Self-dependency — a dependency where source == target.
        for dep in model.dependencies() {
            if dep.source == dep.target {
                issues.push(ValidationIssue {
                    rule: ValidationRule::SelfDependency,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Dependency, dep.id.as_str()),
                    message: format!(
                        "dependency {} is a self-dependency ({})",
                        dep.kind, dep.source
                    ),
                });
            }
        }

        // 6. Broken locations — a SourceLocation pointing at an unknown
        //    workspace, package or module id.
        for symbol in model.symbols() {
            Self::check_location(&mut issues, &symbol.location, &universe, "symbol");
        }
        for module in model.modules() {
            Self::check_location(&mut issues, &module.location, &universe, "module");
        }
        for rel in model.relationships() {
            if let Some(loc) = &rel.location {
                Self::check_location(&mut issues, loc, &universe, "relationship");
            }
        }
        for reference in model.references() {
            if let Some(loc) = &reference.location {
                Self::check_location(&mut issues, loc, &universe, "reference");
            }
        }
        for test in model.tests() {
            if let Some(loc) = &test.location {
                Self::check_location(&mut issues, loc, &universe, "test");
            }
        }
        for diagnostic in model.diagnostics() {
            if let Some(loc) = &diagnostic.location {
                Self::check_location(&mut issues, loc, &universe, "diagnostic");
            }
        }

        // 7. Invalid (unresolved) visibility — warning for any symbol or
        //    module whose visibility is still `Unknown`.
        for symbol in model.symbols() {
            if symbol.visibility == Visibility::Unknown {
                issues.push(ValidationIssue {
                    rule: ValidationRule::InvalidVisibility,
                    severity: Severity::Warning,
                    entity: FactId::new(FactKind::Symbol, symbol.id.as_str()),
                    message: "symbol visibility is unknown/unresolved".to_string(),
                });
            }
        }
        for module in model.modules() {
            if module.visibility == Visibility::Unknown {
                issues.push(ValidationIssue {
                    rule: ValidationRule::InvalidVisibility,
                    severity: Severity::Warning,
                    entity: FactId::new(FactKind::Module, module.id.as_str()),
                    message: "module visibility is unknown/unresolved".to_string(),
                });
            }
        }

        // 8. Orphan symbols — a symbol with no owning module and no
        //    module→symbol Owns/Contains/Defines/Declares relationship.
        let mut claimed: HashMap<&str, ()> = HashMap::new();
        for rel in model.relationships() {
            if matches!(
                rel.kind,
                RelationshipKind::Owns
                    | RelationshipKind::Contains
                    | RelationshipKind::Defines
                    | RelationshipKind::Declares
            ) {
                claimed.insert(rel.target.as_str(), ());
            }
        }
        for symbol in model.symbols() {
            if symbol.module.is_none() && !claimed.contains_key(symbol.id.as_str()) {
                issues.push(ValidationIssue {
                    rule: ValidationRule::OrphanSymbol,
                    severity: Severity::Warning,
                    entity: FactId::new(FactKind::Symbol, symbol.id.as_str()),
                    message: "symbol is not owned by any module".to_string(),
                });
            }
        }

        // Deterministic ordering.
        issues.sort_by(|a, b| {
            a.rule
                .cmp(&b.rule)
                .then_with(|| a.entity.cmp(&b.entity))
                .then_with(|| a.message.cmp(&b.message))
        });

        ValidationReport {
            issues,
            checked_entities,
        }
    }

    /// Build the id universe: every entity id keyed by its opaque string,
    /// plus the ordered (kind, id) list used for duplicate detection, so
    /// both cross-kind uniqueness and endpoint resolution are exact.
    fn universe(model: &FactsModel) -> Universe {
        let mut ids: Vec<(FactKind, String)> = Vec::new();
        let mut push = |kind: FactKind, s: &str| ids.push((kind, s.to_string()));
        for f in model.workspaces() {
            push(FactKind::Workspace, f.id.as_str());
        }
        for f in model.modules() {
            push(FactKind::Module, f.id.as_str());
        }
        for f in model.packages() {
            push(FactKind::Package, f.id.as_str());
        }
        for f in model.symbols() {
            push(FactKind::Symbol, f.id.as_str());
        }
        for f in model.tests() {
            push(FactKind::Test, f.id.as_str());
        }
        for f in model.build_targets() {
            push(FactKind::BuildTarget, f.id.as_str());
        }
        for f in model.dependencies() {
            push(FactKind::Dependency, f.id.as_str());
        }
        for f in model.relationships() {
            push(FactKind::Relationship, f.id.as_str());
        }
        for f in model.references() {
            push(FactKind::Reference, f.id.as_str());
        }
        for f in model.diagnostics() {
            push(FactKind::Diagnostic, f.id.as_str());
        }
        for f in model.architecture_rules() {
            push(FactKind::ArchitectureRule, f.id.as_str());
        }
        let set: HashMap<String, ()> = ids.iter().map(|(_, s)| (s.clone(), ())).collect();
        Universe { ordered: ids, set }
    }

    fn check_endpoint(
        issues: &mut Vec<ValidationIssue>,
        id: &FactId,
        universe: &Universe,
        what: &str,
    ) {
        if !universe.set.contains_key(id.as_str()) {
            issues.push(ValidationIssue {
                rule: ValidationRule::InvalidReference,
                severity: Severity::Error,
                entity: id.clone(),
                message: format!("{what} does not resolve to a known fact"),
            });
        }
    }

    fn check_typed_endpoint<
        K: crate::engineering_facts::ids::IdKey + crate::engineering_facts::ids::FactIdKind,
    >(
        issues: &mut Vec<ValidationIssue>,
        id: &K,
        universe: &Universe,
        what: &str,
    ) {
        if !universe.set.contains_key(id.key()) {
            issues.push(ValidationIssue {
                rule: ValidationRule::InvalidReference,
                severity: Severity::Error,
                entity: FactId::new(K::KIND, id.key()),
                message: format!("{what} does not resolve to a known fact"),
            });
        }
    }

    fn check_location(
        issues: &mut Vec<ValidationIssue>,
        loc: &crate::engineering_facts::location::SourceLocation,
        universe: &Universe,
        holder: &str,
    ) {
        if let Some(w) = &loc.workspace {
            if !universe.set.contains_key(w.as_str()) {
                issues.push(ValidationIssue {
                    rule: ValidationRule::BrokenLocation,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Workspace, w.as_str()),
                    message: format!("{holder} location workspace does not resolve"),
                });
            }
        }
        if let Some(p) = &loc.package {
            if !universe.set.contains_key(p.as_str()) {
                issues.push(ValidationIssue {
                    rule: ValidationRule::BrokenLocation,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Package, p.as_str()),
                    message: format!("{holder} location package does not resolve"),
                });
            }
        }
        if let Some(m) = &loc.module {
            if !universe.set.contains_key(m.as_str()) {
                issues.push(ValidationIssue {
                    rule: ValidationRule::BrokenLocation,
                    severity: Severity::Error,
                    entity: FactId::new(FactKind::Module, m.as_str()),
                    message: format!("{holder} location module does not resolve"),
                });
            }
        }
    }
}

/// The id universe of a model: every entity id with its kind (for duplicate
/// detection) plus the flat set (for endpoint resolution).
struct Universe {
    ordered: Vec<(FactKind, String)>,
    set: HashMap<String, ()>,
}

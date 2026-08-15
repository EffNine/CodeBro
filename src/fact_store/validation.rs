#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Deterministic store validation (P10.5.1).
//!
//! [`FactValidation`] runs a fixed rule set over a frozen [`FactStore`] and
//! produces an immutable, sorted [`FactValidationReport`]:
//!
//! - `DuplicateFacts` — an opaque id occurring more than once across the
//!   collection.
//! - `BrokenIndex` — a primary or reverse index entry referencing an id that
//!   does not resolve in the collection.
//! - `MissingIds` — a collection record absent from the primary index of its
//!   kind (index incompleteness).
//! - `OrphanRecords` — a collection record scoped by no reverse index.
//! - `SchemaMismatch` — a primary index entry whose id kind does not match
//!   the index kind.
//!
//! All checks are deterministic; issues are sorted by
//! `(rule, entity, message)` before emission.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::engineering_facts::{FactId, FactKind, FactRef, Severity};
use crate::fact_store::index::fact_id_of;
use crate::fact_store::store::FactStore;

/// The deterministic store validation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FactValidationRule {
    DuplicateFacts,
    BrokenIndex,
    MissingIds,
    OrphanRecords,
    SchemaMismatch,
}

impl FactValidationRule {
    pub const ALL: [FactValidationRule; 5] = [
        FactValidationRule::DuplicateFacts,
        FactValidationRule::BrokenIndex,
        FactValidationRule::MissingIds,
        FactValidationRule::OrphanRecords,
        FactValidationRule::SchemaMismatch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FactValidationRule::DuplicateFacts => "duplicate_facts",
            FactValidationRule::BrokenIndex => "broken_index",
            FactValidationRule::MissingIds => "missing_ids",
            FactValidationRule::OrphanRecords => "orphan_records",
            FactValidationRule::SchemaMismatch => "schema_mismatch",
        }
    }

    pub fn parse(s: &str) -> Option<FactValidationRule> {
        match s {
            "duplicate_facts" => Some(FactValidationRule::DuplicateFacts),
            "broken_index" => Some(FactValidationRule::BrokenIndex),
            "missing_ids" => Some(FactValidationRule::MissingIds),
            "orphan_records" => Some(FactValidationRule::OrphanRecords),
            "schema_mismatch" => Some(FactValidationRule::SchemaMismatch),
            _ => None,
        }
    }
}

impl std::fmt::Display for FactValidationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single store validation finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FactValidationIssue {
    pub rule: FactValidationRule,
    pub severity: Severity,
    pub entity: FactId,
    pub message: String,
}

/// The immutable, deterministic result of validating a [`FactStore`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactValidationReport {
    /// Issues sorted by `(rule, entity, message)`.
    pub issues: Vec<FactValidationIssue>,
    /// Number of facts considered.
    pub checked_entities: usize,
    /// Number of index entries (primary + reverse) considered.
    pub checked_index_entries: usize,
}

impl FactValidationReport {
    /// True when there are no error or fatal findings. Warnings (e.g.
    /// orphan records) do not fail validation.
    pub fn passed(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
    }

    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }

    pub fn count_by_rule(&self, rule: FactValidationRule) -> usize {
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

/// Applies the fixed store validation rule set to a frozen [`FactStore`].
pub struct FactValidation;

impl FactValidation {
    pub fn validate(store: &FactStore) -> FactValidationReport {
        let collection = store.collection();
        let index = store.index();
        let mut issues: Vec<FactValidationIssue> = Vec::new();

        // 1. Duplicate facts — any opaque id occurring more than once
        //    anywhere in the collection.
        let mut all_ids: Vec<(FactKind, String)> = Vec::new();
        for fact in collection.iter() {
            let id = fact_id_of(&fact);
            all_ids.push((id.kind(), id.as_str().to_string()));
        }
        all_ids.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let mut i = 0;
        while i < all_ids.len() {
            let mut j = i + 1;
            while j < all_ids.len() && all_ids[j].1 == all_ids[i].1 {
                j += 1;
            }
            if j - i > 1 {
                issues.push(FactValidationIssue {
                    rule: FactValidationRule::DuplicateFacts,
                    severity: Severity::Error,
                    entity: FactId::new(all_ids[i].0, &all_ids[i].1),
                    message: format!("id appears {} times in the collection", j - i),
                });
            }
            i = j;
        }

        // 2. Missing ids — every collection record must be present in the
        //    primary index of its kind.
        for fact in collection.iter() {
            let id = fact_id_of(&fact);
            let kind = id.kind();
            if !index.contains_in_kind(kind, &id) {
                issues.push(FactValidationIssue {
                    rule: FactValidationRule::MissingIds,
                    severity: Severity::Error,
                    entity: id,
                    message: format!("{kind} fact is absent from the {kind} primary index"),
                });
            }
        }

        // 3. Broken indexes — every primary and reverse index entry must
        //    resolve in the collection.
        for kind in FactKind::ALL {
            for id in index.facts_of_kind(kind) {
                if !collection.contains(id) {
                    issues.push(FactValidationIssue {
                        rule: FactValidationRule::BrokenIndex,
                        severity: Severity::Error,
                        entity: id.clone(),
                        message: format!(
                            "primary index entry does not resolve to a known fact ({kind})"
                        ),
                    });
                }
            }
        }
        for reverse in [
            index.reverse_workspace(),
            index.reverse_package(),
            index.reverse_module(),
            index.reverse_symbol(),
        ] {
            for pair in reverse.entries() {
                if !collection.contains(&pair.owner) {
                    issues.push(FactValidationIssue {
                        rule: FactValidationRule::BrokenIndex,
                        severity: Severity::Error,
                        entity: pair.owner.clone(),
                        message: "reverse index owner does not resolve to a known fact".to_string(),
                    });
                }
                if !collection.contains(&pair.member) {
                    issues.push(FactValidationIssue {
                        rule: FactValidationRule::BrokenIndex,
                        severity: Severity::Error,
                        entity: pair.member.clone(),
                        message: "reverse index member does not resolve to a known fact"
                            .to_string(),
                    });
                }
            }
        }

        // 4. Schema consistency — every primary index entry's id kind must
        //    match the index kind.
        for kind in FactKind::ALL {
            for id in index.facts_of_kind(kind) {
                if id.kind() != kind {
                    issues.push(FactValidationIssue {
                        rule: FactValidationRule::SchemaMismatch,
                        severity: Severity::Error,
                        entity: id.clone(),
                        message: format!("{kind} primary index holds a {} id", id.kind()),
                    });
                }
            }
        }

        // 5. Orphan records — a collection record scoped by no reverse
        //    index. Workspaces are the root container and are exempt;
        //    dependency facts carry no scope projection in the model (no
        //    location and no owner field), so they are exempt too; and
        //    external (workspace-less) package facts — e.g. third-party
        //    crates referenced by dependency links — are external entities
        //    that are intentionally not scoped inside this workspace.
        let mut scoped: HashSet<FactId> = HashSet::new();
        for reverse in [
            index.reverse_workspace(),
            index.reverse_package(),
            index.reverse_module(),
            index.reverse_symbol(),
        ] {
            for pair in reverse.entries() {
                scoped.insert(pair.member.clone());
            }
        }
        for fact in collection.iter() {
            let is_external_package = matches!(fact, FactRef::Package(p) if p.workspace.is_none());
            if matches!(fact, FactRef::Workspace(_) | FactRef::Dependency(_))
                || is_external_package
            {
                continue;
            }
            let id = fact_id_of(&fact);
            if !scoped.contains(&id) {
                issues.push(FactValidationIssue {
                    rule: FactValidationRule::OrphanRecords,
                    severity: Severity::Warning,
                    entity: id,
                    message: "record is scoped by no reverse index".to_string(),
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

        FactValidationReport {
            issues,
            checked_entities: all_ids.len(),
            checked_index_entries: index.primary_len() + index.reverse_len(),
        }
    }
}

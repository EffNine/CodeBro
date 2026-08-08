#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Relationship and reference facts (P10.5.0).
//!
//! Relationships are the directed edges of the engineering knowledge graph:
//! `source --kind--> target`. Every relationship is directional and
//! classified by a language-neutral `RelationshipKind`. References are a
//! specialised, location-bearing relationship for symbol resolution.
//!
//! The supported relationship kinds are fully language-neutral:
//! `Defines`, `Declares`, `Calls`, `References`, `Imports`, `Exports`,
//! `DependsOn`, `Contains`, `Owns`, `Implements`, `Overrides`, `Tests`,
//! `Builds`, `Friend` and `Unknown`.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{FactId, ReferenceId, RelationshipId};
use crate::engineering_facts::location::SourceLocation;
use crate::engineering_facts::metadata::FactMetadata;

/// Language-neutral relationship categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RelationshipKind {
    Defines,
    Declares,
    Calls,
    References,
    Imports,
    Exports,
    DependsOn,
    Implements,
    Overrides,
    Tests,
    Builds,
    Owns,
    Contains,
    Friend,
    Unknown,
}

impl RelationshipKind {
    pub const ALL: [RelationshipKind; 15] = [
        RelationshipKind::Defines,
        RelationshipKind::Declares,
        RelationshipKind::Calls,
        RelationshipKind::References,
        RelationshipKind::Imports,
        RelationshipKind::Exports,
        RelationshipKind::DependsOn,
        RelationshipKind::Implements,
        RelationshipKind::Overrides,
        RelationshipKind::Tests,
        RelationshipKind::Builds,
        RelationshipKind::Owns,
        RelationshipKind::Contains,
        RelationshipKind::Friend,
        RelationshipKind::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RelationshipKind::Defines => "defines",
            RelationshipKind::Declares => "declares",
            RelationshipKind::Calls => "calls",
            RelationshipKind::References => "references",
            RelationshipKind::Imports => "imports",
            RelationshipKind::Exports => "exports",
            RelationshipKind::DependsOn => "depends_on",
            RelationshipKind::Implements => "implements",
            RelationshipKind::Overrides => "overrides",
            RelationshipKind::Tests => "tests",
            RelationshipKind::Builds => "builds",
            RelationshipKind::Owns => "owns",
            RelationshipKind::Contains => "contains",
            RelationshipKind::Friend => "friend",
            RelationshipKind::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<RelationshipKind> {
        match s {
            "defines" => Some(RelationshipKind::Defines),
            "declares" => Some(RelationshipKind::Declares),
            "calls" => Some(RelationshipKind::Calls),
            "references" => Some(RelationshipKind::References),
            "imports" => Some(RelationshipKind::Imports),
            "exports" => Some(RelationshipKind::Exports),
            "depends_on" => Some(RelationshipKind::DependsOn),
            "implements" => Some(RelationshipKind::Implements),
            "overrides" => Some(RelationshipKind::Overrides),
            "tests" => Some(RelationshipKind::Tests),
            "builds" => Some(RelationshipKind::Builds),
            "owns" => Some(RelationshipKind::Owns),
            "contains" => Some(RelationshipKind::Contains),
            "friend" => Some(RelationshipKind::Friend),
            "unknown" => Some(RelationshipKind::Unknown),
            _ => None,
        }
    }
}

impl std::fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A directed relationship `source --kind--> target`. Immutable. Owned by
/// the Engineering Facts Model. Always directional; endpoints reference any
/// fact kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipFact {
    pub id: RelationshipId,
    pub kind: RelationshipKind,
    pub source: FactId,
    pub target: FactId,
    pub location: Option<SourceLocation>,
    pub metadata: FactMetadata,
}

impl RelationshipFact {
    pub fn new(id: RelationshipId, kind: RelationshipKind, source: FactId, target: FactId) -> Self {
        RelationshipFact {
            id,
            kind,
            source,
            target,
            location: None,
            metadata: FactMetadata::new(),
        }
    }
}

/// A directed reference `referrer → target` with a location. This is the
/// fact a resolver uses to answer "what references X?" and "where is Y
/// referenced?". Immutable. Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceFact {
    pub id: ReferenceId,
    /// The entity that contains the reference.
    pub referrer: FactId,
    /// The referenced entity.
    pub target: FactId,
    pub location: Option<SourceLocation>,
    pub metadata: FactMetadata,
}

impl ReferenceFact {
    pub fn new(id: ReferenceId, referrer: FactId, target: FactId) -> Self {
        ReferenceFact {
            id,
            referrer,
            target,
            location: None,
            metadata: FactMetadata::new(),
        }
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Dependency facts (P10.5.0).
//!
//! A dependency is a directed link from a depending entity to a dependency
//! entity (e.g. package → package, module → module). Dependencies are pure
//! knowledge about versioned relationships — never about how source is
//! parsed.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{DependencyId, FactId};
use crate::engineering_facts::metadata::FactMetadata;

/// Language-neutral dependency categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DependencyKind {
    Direct,
    Transitive,
    Optional,
    Dev,
    Test,
    Build,
    Runtime,
    Peer,
    Unknown,
}

impl DependencyKind {
    pub const ALL: [DependencyKind; 9] = [
        DependencyKind::Direct,
        DependencyKind::Transitive,
        DependencyKind::Optional,
        DependencyKind::Dev,
        DependencyKind::Test,
        DependencyKind::Build,
        DependencyKind::Runtime,
        DependencyKind::Peer,
        DependencyKind::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DependencyKind::Direct => "direct",
            DependencyKind::Transitive => "transitive",
            DependencyKind::Optional => "optional",
            DependencyKind::Dev => "dev",
            DependencyKind::Test => "test",
            DependencyKind::Build => "build",
            DependencyKind::Runtime => "runtime",
            DependencyKind::Peer => "peer",
            DependencyKind::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<DependencyKind> {
        match s {
            "direct" => Some(DependencyKind::Direct),
            "transitive" => Some(DependencyKind::Transitive),
            "optional" => Some(DependencyKind::Optional),
            "dev" => Some(DependencyKind::Dev),
            "test" => Some(DependencyKind::Test),
            "build" => Some(DependencyKind::Build),
            "runtime" => Some(DependencyKind::Runtime),
            "peer" => Some(DependencyKind::Peer),
            "unknown" => Some(DependencyKind::Unknown),
            _ => None,
        }
    }
}

impl std::fmt::Display for DependencyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A directed dependency link `source → target`. Immutable. Owned by the
/// Engineering Facts Model. Endpoints reference any fact kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyFact {
    pub id: DependencyId,
    pub kind: DependencyKind,
    /// The entity that depends on `target`.
    pub source: FactId,
    /// The entity `source` depends on.
    pub target: FactId,
    /// Optional declared version constraint.
    pub version_constraint: Option<String>,
    pub metadata: FactMetadata,
}

impl DependencyFact {
    pub fn new(id: DependencyId, source: FactId, target: FactId) -> Self {
        DependencyFact {
            id,
            kind: DependencyKind::Direct,
            source,
            target,
            version_constraint: None,
            metadata: FactMetadata::new(),
        }
    }
}

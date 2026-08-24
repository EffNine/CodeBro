#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Build target facts (P10.5.0).
//!
//! A build target is a buildable product declared by a package: a binary, a
//! library, a test, an example or a bench. Language-neutral product kinds —
//! never a compiler or build-system concept.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{BuildTargetId, PackageId};
use crate::engineering_facts::metadata::FactMetadata;

/// Build target categories — language-neutral product kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BuildTargetKind {
    Binary,
    Library,
    Test,
    Example,
    Bench,
    Unknown,
}

impl BuildTargetKind {
    pub const ALL: [BuildTargetKind; 6] = [
        BuildTargetKind::Binary,
        BuildTargetKind::Library,
        BuildTargetKind::Test,
        BuildTargetKind::Example,
        BuildTargetKind::Bench,
        BuildTargetKind::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BuildTargetKind::Binary => "binary",
            BuildTargetKind::Library => "library",
            BuildTargetKind::Test => "test",
            BuildTargetKind::Example => "example",
            BuildTargetKind::Bench => "bench",
            BuildTargetKind::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<BuildTargetKind> {
        match s {
            "binary" => Some(BuildTargetKind::Binary),
            "library" => Some(BuildTargetKind::Library),
            "test" => Some(BuildTargetKind::Test),
            "example" => Some(BuildTargetKind::Example),
            "bench" => Some(BuildTargetKind::Bench),
            "unknown" => Some(BuildTargetKind::Unknown),
            _ => None,
        }
    }
}

impl std::fmt::Display for BuildTargetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A build target fact — a buildable product declared by the workspace.
/// Immutable. Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildTargetFact {
    pub id: BuildTargetId,
    pub name: String,
    pub kind: BuildTargetKind,
    pub language: Option<String>,
    pub package: Option<PackageId>,
    pub metadata: FactMetadata,
}

impl BuildTargetFact {
    pub fn new(id: BuildTargetId, name: impl Into<String>, kind: BuildTargetKind) -> Self {
        BuildTargetFact {
            id,
            name: name.into(),
            kind,
            language: None,
            package: None,
            metadata: FactMetadata::new(),
        }
    }
}

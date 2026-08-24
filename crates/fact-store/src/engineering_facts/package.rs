#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Package and workspace facts (P10.5.0).
//!
//! These two facts describe the structural containers of engineering
//! knowledge. A package is a distributable unit that groups modules and
//! build targets; a workspace is the top-level container of packages.
//! Nothing here reads or parses source.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{BuildTargetId, PackageId, WorkspaceId};
use crate::engineering_facts::metadata::FactMetadata;

/// A package fact — a distributable unit that groups modules and build
/// targets. Immutable. Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFact {
    pub id: PackageId,
    pub name: String,
    pub version: Option<String>,
    /// Owning workspace, if any.
    pub workspace: Option<WorkspaceId>,
    /// Optional producer-declared language tag (informational only).
    pub language: Option<String>,
    /// Build targets owned by this package.
    pub build_targets: Vec<BuildTargetId>,
    pub metadata: FactMetadata,
}

impl PackageFact {
    pub fn new(id: PackageId, name: impl Into<String>) -> Self {
        PackageFact {
            id,
            name: name.into(),
            version: None,
            workspace: None,
            language: None,
            build_targets: Vec::new(),
            metadata: FactMetadata::new(),
        }
    }
}

/// A workspace fact — the top-level container of packages. Immutable.
/// Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFact {
    pub id: WorkspaceId,
    pub name: String,
    /// Optional workspace root path (informational only).
    pub root: Option<String>,
    /// Packages belonging to this workspace.
    pub packages: Vec<PackageId>,
    pub metadata: FactMetadata,
}

impl WorkspaceFact {
    pub fn new(id: WorkspaceId, name: impl Into<String>) -> Self {
        WorkspaceFact {
            id,
            name: name.into(),
            root: None,
            packages: Vec::new(),
            metadata: FactMetadata::new(),
        }
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Module facts (P10.5.0).
//!
//! A module is a named, addressable unit of engineering knowledge. It lives
//! in a package and exposes an API surface. Nothing here reads or parses
//! source.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{ModuleId, PackageId};
use crate::engineering_facts::location::SourceLocation;
use crate::engineering_facts::metadata::FactMetadata;
use crate::engineering_facts::symbol::ApiSurface;
use crate::engineering_facts::visibility::Visibility;

/// A module fact — a named, addressable unit of engineering knowledge.
/// Immutable. Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFact {
    pub id: ModuleId,
    pub name: String,
    /// Owning package, if any.
    pub package: Option<PackageId>,
    /// Canonical path within the workspace (e.g. `src/lib.rs`).
    pub path: Option<String>,
    pub visibility: Visibility,
    pub location: SourceLocation,
    pub api: ApiSurface,
    pub metadata: FactMetadata,
}

impl ModuleFact {
    pub fn new(id: ModuleId, name: impl Into<String>) -> Self {
        ModuleFact {
            id,
            name: name.into(),
            package: None,
            path: None,
            visibility: Visibility::Public,
            location: SourceLocation::new(),
            api: ApiSurface::empty(),
            metadata: FactMetadata::new(),
        }
    }
}

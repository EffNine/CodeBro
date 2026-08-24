#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Framework facts — detected frameworks and their evidence.
//!
//! A framework fact exists only when a concrete piece of evidence (a
//! declared dependency or manifest key) proves it. Detection is curated
//! per ecosystem; unknown dependencies never invent frameworks.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::FrameworkId;
use crate::engineering_facts::metadata::FactMetadata;

/// A framework detected in the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkFact {
    pub id: FrameworkId,
    /// Human framework name ("Axum", "FastAPI", "Express").
    pub name: String,
    /// Package-manager ecosystem the evidence came from.
    pub ecosystem: String,
    /// The dependency name / manifest key that proved this framework.
    pub evidence: String,
    /// Owning package when the evidence is package-scoped; `None` for
    /// repository-level detections.
    pub scope_package: Option<crate::engineering_facts::ids::PackageId>,
    pub metadata: FactMetadata,
}

impl FrameworkFact {
    pub fn new(
        id: FrameworkId,
        name: impl Into<String>,
        ecosystem: impl Into<String>,
        evidence: impl Into<String>,
    ) -> Self {
        FrameworkFact {
            id,
            name: name.into(),
            ecosystem: ecosystem.into(),
            evidence: evidence.into(),
            scope_package: None,
            metadata: FactMetadata::new(),
        }
    }
}

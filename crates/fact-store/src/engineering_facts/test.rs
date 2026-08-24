#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Test facts (P10.5.0).
//!
//! A test fact maps a test to the module or build target it belongs to and
//! the symbols whose behaviour it exercises. Pure engineering knowledge —
//! no execution, no telemetry.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{FactId, SymbolId, TestId};
use crate::engineering_facts::location::SourceLocation;
use crate::engineering_facts::metadata::FactMetadata;

/// A test fact — maps a test to the module and symbols whose behaviour it
/// exercises. Immutable. Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFact {
    pub id: TestId,
    pub name: String,
    /// The module or build target the test belongs to.
    pub target: Option<FactId>,
    /// The symbols this test exercises.
    pub tested: Vec<SymbolId>,
    pub location: Option<SourceLocation>,
    pub metadata: FactMetadata,
}

impl TestFact {
    pub fn new(id: TestId, name: impl Into<String>) -> Self {
        TestFact {
            id,
            name: name.into(),
            target: None,
            tested: Vec::new(),
            location: None,
            metadata: FactMetadata::new(),
        }
    }
}

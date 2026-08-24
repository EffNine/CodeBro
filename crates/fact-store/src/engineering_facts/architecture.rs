#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Architecture rule facts (P10.5.0).
//!
//! An architecture rule is a declared boundary or layering constraint over
//! engineering facts. Rules are pure declarations; enforcement is a
//! downstream consumer's responsibility.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{ArchitectureRuleId, FactId};
use crate::engineering_facts::metadata::FactMetadata;

/// An architecture rule fact — a declared boundary or layering constraint.
/// Immutable. Owned by the Engineering Facts Model. Rules are pure
/// declarations; enforcement is a downstream consumer's responsibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureRuleFact {
    pub id: ArchitectureRuleId,
    pub name: String,
    pub from: Option<FactId>,
    pub to: Option<FactId>,
    pub description: Option<String>,
    pub metadata: FactMetadata,
}

impl ArchitectureRuleFact {
    pub fn new(id: ArchitectureRuleId, name: impl Into<String>) -> Self {
        ArchitectureRuleFact {
            id,
            name: name.into(),
            from: None,
            to: None,
            description: None,
            metadata: FactMetadata::new(),
        }
    }
}

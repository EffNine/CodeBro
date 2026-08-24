#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Strongly-typed opaque IDs (P10.5.0).
//!
//! Every engineering fact is addressed by an **opaque, strongly-typed id**.
//! There is one id type per entity: `WorkspaceId`, `PackageId`, `ModuleId`,
//! `SymbolId`, `TestId`, `BuildTargetId`, `DependencyId`, `RelationshipId`,
//! `ReferenceId`, `DiagnosticId` and `ArchitectureRuleId`.
//!
//! IDs are producer-supplied opaque strings. There is **no UUID generation,
//! no timestamp and no randomness** anywhere in the model: an id is exactly
//! the string a producer assigned. The model never constructs ids itself.
//!
//! [`FactId`] is the union reference type used where a fact may point at any
//! kind of entity (relationship endpoints, reference endpoints, diagnostic
//! `related`, architecture-rule bounds). Typed ids convert into `FactId`
//! without loss.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::types::FactKind;

/// Uniform, allocation-free accessor for the opaque id payload.
pub trait IdKey {
    fn key(&self) -> &str;
}

/// Maps an id type to its [`FactKind`], used to build `FactId` unions.
pub trait FactIdKind: IdKey {
    const KIND: FactKind;
}

macro_rules! opaque_id {
    ($(#[$doc:meta])* $name:ident, $kind:path) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wrap a producer-supplied id string.
            pub fn new(inner: impl Into<String>) -> Self {
                Self(inner.into())
            }

            /// View the underlying opaque string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl IdKey for $name {
            fn key(&self) -> &str {
                &self.0
            }
        }

        impl FactIdKind for $name {
            const KIND: FactKind = $kind;
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<$name> for FactId {
            fn from(id: $name) -> Self {
                FactId::new(<$name>::KIND, id.as_str())
            }
        }

        impl From<&$name> for FactId {
            fn from(id: &$name) -> Self {
                FactId::new(<$name>::KIND, id.as_str())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id!(/// Opaque id of a `WorkspaceFact`.
    WorkspaceId, FactKind::Workspace);
opaque_id!(/// Opaque id of a `PackageFact`.
    PackageId, FactKind::Package);
opaque_id!(/// Opaque id of a `ModuleFact`.
    ModuleId, FactKind::Module);
opaque_id!(/// Opaque id of a `SymbolFact`.
    SymbolId, FactKind::Symbol);
opaque_id!(/// Opaque id of a `TestFact`.
    TestId, FactKind::Test);
opaque_id!(/// Opaque id of a `BuildTargetFact`.
    BuildTargetId, FactKind::BuildTarget);
opaque_id!(/// Opaque id of a `DependencyFact`.
    DependencyId, FactKind::Dependency);
opaque_id!(/// Opaque id of a `RelationshipFact`.
    RelationshipId, FactKind::Relationship);
opaque_id!(/// Opaque id of a `ReferenceFact`.
    ReferenceId, FactKind::Reference);
opaque_id!(/// Opaque id of a `DiagnosticFact`.
    DiagnosticId, FactKind::Diagnostic);
opaque_id!(/// Opaque id of an `ArchitectureRuleFact`.
    ArchitectureRuleId, FactKind::ArchitectureRule);

/// A union id referencing any engineering fact, used by cross-entity links
/// (relationship and reference endpoints, diagnostic `related`, architecture
/// rule bounds). Immutable, `Eq`, `Hash`, ordered and serde-serialisable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FactId {
    Workspace(WorkspaceId),
    Package(PackageId),
    Module(ModuleId),
    Symbol(SymbolId),
    Test(TestId),
    BuildTarget(BuildTargetId),
    Dependency(DependencyId),
    Relationship(RelationshipId),
    Reference(ReferenceId),
    Diagnostic(DiagnosticId),
    ArchitectureRule(ArchitectureRuleId),
}

impl FactId {
    /// Build a union id from a kind and a producer-supplied opaque value.
    pub fn new(kind: FactKind, value: &str) -> Self {
        match kind {
            FactKind::Workspace => FactId::Workspace(WorkspaceId::new(value)),
            FactKind::Package => FactId::Package(PackageId::new(value)),
            FactKind::Module => FactId::Module(ModuleId::new(value)),
            FactKind::Symbol => FactId::Symbol(SymbolId::new(value)),
            FactKind::Test => FactId::Test(TestId::new(value)),
            FactKind::BuildTarget => FactId::BuildTarget(BuildTargetId::new(value)),
            FactKind::Dependency => FactId::Dependency(DependencyId::new(value)),
            FactKind::Relationship => FactId::Relationship(RelationshipId::new(value)),
            FactKind::Reference => FactId::Reference(ReferenceId::new(value)),
            FactKind::Diagnostic => FactId::Diagnostic(DiagnosticId::new(value)),
            FactKind::ArchitectureRule => FactId::ArchitectureRule(ArchitectureRuleId::new(value)),
        }
    }

    /// The entity kind this id addresses.
    pub fn kind(&self) -> FactKind {
        match self {
            FactId::Workspace(_) => FactKind::Workspace,
            FactId::Package(_) => FactKind::Package,
            FactId::Module(_) => FactKind::Module,
            FactId::Symbol(_) => FactKind::Symbol,
            FactId::Test(_) => FactKind::Test,
            FactId::BuildTarget(_) => FactKind::BuildTarget,
            FactId::Dependency(_) => FactKind::Dependency,
            FactId::Relationship(_) => FactKind::Relationship,
            FactId::Reference(_) => FactKind::Reference,
            FactId::Diagnostic(_) => FactKind::Diagnostic,
            FactId::ArchitectureRule(_) => FactKind::ArchitectureRule,
        }
    }

    /// The underlying opaque string.
    pub fn as_str(&self) -> &str {
        match self {
            FactId::Workspace(id) => id.as_str(),
            FactId::Package(id) => id.as_str(),
            FactId::Module(id) => id.as_str(),
            FactId::Symbol(id) => id.as_str(),
            FactId::Test(id) => id.as_str(),
            FactId::BuildTarget(id) => id.as_str(),
            FactId::Dependency(id) => id.as_str(),
            FactId::Relationship(id) => id.as_str(),
            FactId::Reference(id) => id.as_str(),
            FactId::Diagnostic(id) => id.as_str(),
            FactId::ArchitectureRule(id) => id.as_str(),
        }
    }
}

impl IdKey for FactId {
    fn key(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for FactId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for FactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

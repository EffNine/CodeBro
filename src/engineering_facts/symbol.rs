#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Symbol facts (P10.5.0).
//!
//! A symbol is the unit of engineering knowledge about a named entity: its
//! role (`SymbolKind`), visibility, owning module, source location and
//! signature. Kinds are language-neutral engineering categories — never
//! syntax or AST shapes.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{ModuleId, SymbolId};
use crate::engineering_facts::location::SourceLocation;
use crate::engineering_facts::metadata::FactMetadata;
use crate::engineering_facts::visibility::Visibility;

/// Language-neutral symbol role categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    TypeAlias,
    Variable,
    Constant,
    Field,
    Parameter,
    Macro,
    Constructor,
    Operator,
    Namespace,
    Import,
    Export,
    Unknown,
}

impl SymbolKind {
    pub const ALL: [SymbolKind; 19] = [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Enum,
        SymbolKind::Trait,
        SymbolKind::Interface,
        SymbolKind::TypeAlias,
        SymbolKind::Variable,
        SymbolKind::Constant,
        SymbolKind::Field,
        SymbolKind::Parameter,
        SymbolKind::Macro,
        SymbolKind::Constructor,
        SymbolKind::Operator,
        SymbolKind::Namespace,
        SymbolKind::Import,
        SymbolKind::Export,
        SymbolKind::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "constant",
            SymbolKind::Field => "field",
            SymbolKind::Parameter => "parameter",
            SymbolKind::Macro => "macro",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Operator => "operator",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Import => "import",
            SymbolKind::Export => "export",
            SymbolKind::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<SymbolKind> {
        match s {
            "function" => Some(SymbolKind::Function),
            "method" => Some(SymbolKind::Method),
            "class" => Some(SymbolKind::Class),
            "struct" => Some(SymbolKind::Struct),
            "enum" => Some(SymbolKind::Enum),
            "trait" => Some(SymbolKind::Trait),
            "interface" => Some(SymbolKind::Interface),
            "type_alias" => Some(SymbolKind::TypeAlias),
            "variable" => Some(SymbolKind::Variable),
            "constant" => Some(SymbolKind::Constant),
            "field" => Some(SymbolKind::Field),
            "parameter" => Some(SymbolKind::Parameter),
            "macro" => Some(SymbolKind::Macro),
            "constructor" => Some(SymbolKind::Constructor),
            "operator" => Some(SymbolKind::Operator),
            "namespace" => Some(SymbolKind::Namespace),
            "import" => Some(SymbolKind::Import),
            "export" => Some(SymbolKind::Export),
            "unknown" => Some(SymbolKind::Unknown),
            _ => None,
        }
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The API surface of a module or package — the opaque symbol ids it
/// exposes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApiSurface {
    /// Symbol ids this module or package exposes publicly.
    pub exports: Vec<SymbolId>,
    /// Entry-point symbol ids (e.g. a crate root, a main, a service).
    pub entry_points: Vec<SymbolId>,
}

impl ApiSurface {
    pub fn empty() -> Self {
        ApiSurface::default()
    }

    pub fn is_empty(&self) -> bool {
        self.exports.is_empty() && self.entry_points.is_empty()
    }

    pub fn exports(&self) -> &[SymbolId] {
        &self.exports
    }

    pub fn entry_points(&self) -> &[SymbolId] {
        &self.entry_points
    }
}

/// A symbol fact. Immutable. Owned by the Engineering Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolFact {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub location: SourceLocation,
    /// Owning module, if known.
    pub module: Option<ModuleId>,
    /// Optional human/engineering signature (not source syntax).
    pub signature: Option<String>,
    pub metadata: FactMetadata,
}

impl SymbolFact {
    pub fn new(id: SymbolId, name: impl Into<String>, kind: SymbolKind) -> Self {
        SymbolFact {
            id,
            name: name.into(),
            kind,
            visibility: Visibility::Unknown,
            location: SourceLocation::new(),
            module: None,
            signature: None,
            metadata: FactMetadata::new(),
        }
    }
}

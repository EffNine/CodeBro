#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Entry-point facts — where execution starts.
//!
//! Detected from manifest declarations (`[[bin]]`, `package.json` `bin`
//! fields, `[project.scripts]`) and language entry conventions
//! (`fn main`, `package main`, `__main__.py`). Deterministic.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::EntryPointId;
use crate::engineering_facts::metadata::FactMetadata;

/// The kind of entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EntryPointKind {
    Binary,
    Script,
    Unknown,
}

impl EntryPointKind {
    pub const ALL: [EntryPointKind; 3] = [
        EntryPointKind::Binary,
        EntryPointKind::Script,
        EntryPointKind::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EntryPointKind::Binary => "binary",
            EntryPointKind::Script => "script",
            EntryPointKind::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<EntryPointKind> {
        match s {
            "binary" => Some(EntryPointKind::Binary),
            "script" => Some(EntryPointKind::Script),
            "unknown" => Some(EntryPointKind::Unknown),
            _ => None,
        }
    }
}

/// A program entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPointFact {
    pub id: EntryPointId,
    /// Display name (binary/script name).
    pub name: String,
    /// Workspace-relative file path of the entry file.
    pub path: String,
    pub kind: EntryPointKind,
    pub language: String,
    /// Owning package, when resolved from a package-scoped manifest.
    pub package: Option<crate::engineering_facts::ids::PackageId>,
    pub metadata: FactMetadata,
}

impl EntryPointFact {
    pub fn new(
        id: EntryPointId,
        name: impl Into<String>,
        path: impl Into<String>,
        kind: EntryPointKind,
        language: impl Into<String>,
    ) -> Self {
        EntryPointFact {
            id,
            name: name.into(),
            path: path.into(),
            kind,
            language: language.into(),
            package: None,
            metadata: FactMetadata::new(),
        }
    }
}

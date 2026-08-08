#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Core shared types for the Engineering Facts Model (P10.5.0).
//!
//! This module defines the shared vocabulary every fact uses: the
//! language-neutral `FactKind` category index and the `Severity` used by
//! diagnostics and validation. Everything here is immutable, deterministic
//! and free of parser, AST and compiler concerns.

use serde::{Deserialize, Serialize};

/// The category of a fact entity. Used for id indexing, validation and
/// diagnostics. The list mirrors the entity ownership model exactly and is
/// language-neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FactKind {
    Workspace,
    Module,
    Package,
    Symbol,
    Test,
    BuildTarget,
    Dependency,
    Relationship,
    Reference,
    Diagnostic,
    ArchitectureRule,
}

impl FactKind {
    /// All known entity categories, in a stable order.
    pub const ALL: [FactKind; 11] = [
        FactKind::Workspace,
        FactKind::Module,
        FactKind::Package,
        FactKind::Symbol,
        FactKind::Test,
        FactKind::BuildTarget,
        FactKind::Dependency,
        FactKind::Relationship,
        FactKind::Reference,
        FactKind::Diagnostic,
        FactKind::ArchitectureRule,
    ];

    /// Canonical snake_case name.
    pub fn as_str(self) -> &'static str {
        match self {
            FactKind::Workspace => "workspace",
            FactKind::Module => "module",
            FactKind::Package => "package",
            FactKind::Symbol => "symbol",
            FactKind::Test => "test",
            FactKind::BuildTarget => "build_target",
            FactKind::Dependency => "dependency",
            FactKind::Relationship => "relationship",
            FactKind::Reference => "reference",
            FactKind::Diagnostic => "diagnostic",
            FactKind::ArchitectureRule => "architecture_rule",
        }
    }

    /// Parse a canonical name back into a category. Unknown strings map to
    /// `None`; there is no catch-all `Unknown` kind for categories.
    pub fn parse(s: &str) -> Option<FactKind> {
        match s {
            "workspace" => Some(FactKind::Workspace),
            "module" => Some(FactKind::Module),
            "package" => Some(FactKind::Package),
            "symbol" => Some(FactKind::Symbol),
            "test" => Some(FactKind::Test),
            "build_target" => Some(FactKind::BuildTarget),
            "dependency" => Some(FactKind::Dependency),
            "relationship" => Some(FactKind::Relationship),
            "reference" => Some(FactKind::Reference),
            "diagnostic" => Some(FactKind::Diagnostic),
            "architecture_rule" => Some(FactKind::ArchitectureRule),
            _ => None,
        }
    }
}

impl std::fmt::Display for FactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Severity of a diagnostic or validation finding. Language-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    pub const ALL: [Severity; 4] = [
        Severity::Info,
        Severity::Warning,
        Severity::Error,
        Severity::Fatal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Fatal => "fatal",
        }
    }

    pub fn parse(s: &str) -> Option<Severity> {
        match s {
            "info" => Some(Severity::Info),
            "warning" => Some(Severity::Warning),
            "error" => Some(Severity::Error),
            "fatal" => Some(Severity::Fatal),
            _ => None,
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

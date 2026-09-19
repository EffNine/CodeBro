#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Source locations in the Engineering Facts Model (P10.5.0).
//!
//! A location is pure engineering knowledge: which workspace, package and
//! module a fact lives in, plus an optional file, direct line/column point
//! and span. Positions are producer-supplied integers — there is no parser,
//! AST or token data and nothing here can ever parse source.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{ModuleId, PackageId, WorkspaceId};

/// A 1-based line/column position. Integers only — no parser dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl Position {
    pub fn new(line: u32, column: u32) -> Self {
        Position { line, column }
    }
}

/// A source span delimited by two positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Span {
    pub fn new(start: Position, end: Position) -> Self {
        Span { start, end }
    }
}

/// A location within the engineering knowledge of a workspace.
///
/// Fields are optional so a producer can attach exactly as much context as
/// it has. `workspace`, `package` and `module` reference facts by opaque id;
/// `file` is a canonical workspace-relative path string; `line`/`column`
/// are a direct 1-based point; `span` is optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourceLocation {
    pub workspace: Option<WorkspaceId>,
    pub package: Option<PackageId>,
    pub module: Option<ModuleId>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub span: Option<Span>,
}

impl SourceLocation {
    /// An empty location.
    pub fn new() -> Self {
        SourceLocation::default()
    }

    /// A location carrying only a canonical file path.
    pub fn file(file: impl Into<String>) -> Self {
        SourceLocation {
            workspace: None,
            package: None,
            module: None,
            file: Some(file.into()),
            line: None,
            column: None,
            span: None,
        }
    }

    /// Attach a workspace id. Returns a new immutable location.
    pub fn with_workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Attach a package id. Returns a new immutable location.
    pub fn with_package(mut self, package: PackageId) -> Self {
        self.package = Some(package);
        self
    }

    /// Attach a module id. Returns a new immutable location.
    pub fn with_module(mut self, module: ModuleId) -> Self {
        self.module = Some(module);
        self
    }

    /// Attach a file path. Returns a new immutable location.
    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Attach a direct line/column point. Returns a new immutable location.
    pub fn with_point(mut self, line: u32, column: u32) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Attach a span. Returns a new immutable location.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// True when no workspace, package, module, file, point or span is
    /// present.
    pub fn is_empty(&self) -> bool {
        self.workspace.is_none()
            && self.package.is_none()
            && self.module.is_none()
            && self.file.is_none()
            && self.line.is_none()
            && self.column.is_none()
            && self.span.is_none()
    }
}

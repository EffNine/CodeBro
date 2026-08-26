//! Intelligence Platform - Read-only code understanding layer.
//!
//! This module provides the foundation for code intelligence within CodeBro.
//! It is strictly read-only: it never writes files or executes commands.
//!
//! ## Components
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `parser` | Tree-sitter based code parsing |
//! | `index` | Symbol indexing and storage |
//! | `graph` | Dependency graph construction |
//! | `search` | Semantic symbol search |
//! | `context` | Context assembly for agents |
//! | `reasoning` | Pre-modification analysis |
//! | `lsp` | LSP protocol foundation types |
//! | `diagnostics` | Platform health monitoring |

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement
pub mod diagnostics;
pub mod parser;

// =========================================================================
// Re-exports
// =========================================================================

pub use diagnostics::{
    ContextMetric, GraphEvent, GraphIntegrity, GraphIntegrityStatus, IndexEvent, IndexHealth,
    IndexHealthStatus, IntelligenceDiagnostics, IntelligenceDiagnosticsTrait, ParseMetric,
    SearchMetric,
};
pub use parser::{
    create_parser, create_parser_trait, parse_file, parse_source, CodeParserTrait, ParseResult,
    ParsedSymbol, ParserSymbolKind, TreeSitterParser,
};

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
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
//! | `memory` | Project intelligence persistence |
//! | `lsp` | LSP protocol foundation types |
//! | `diagnostics` | Platform health monitoring |

pub mod context;
pub mod diagnostics;
pub mod graph;
pub mod index;
pub mod lsp;
pub mod memory;
pub mod parser;
pub mod reasoning;
pub mod search;

// =========================================================================
// Re-exports
// =========================================================================

pub use context::{CodeSnippet, IntelligenceContext, IntelligentContextBuilder};
pub use diagnostics::{
    ContextMetric, GraphEvent, GraphIntegrity, GraphIntegrityStatus, IndexEvent, IndexHealth,
    IndexHealthStatus, IntelligenceDiagnostics, IntelligenceDiagnosticsTrait, ParseMetric,
    SearchMetric,
};
pub use graph::{DependencyGraph, DependencyGraphTrait, DependencyNode};
pub use index::{
    CodeIndexer, CodeIndexerTrait, FileInfo, Symbol, SymbolDatabase, SymbolDatabaseTrait,
    SymbolKind, SymbolRelationship,
};
pub use lsp::{
    create_lsp_foundation, DiagnosticSeverity, LspDiagnostic, LspFoundation, LspFoundationTrait,
    LspHover, LspLocation, LspParameterInformation, LspPosition, LspRange, LspSignatureInformation,
    LspSymbolInformation, LspSymbolKind, LspTextDocumentIdentifier, LspTextDocumentItem,
    LspTextEdit, LspWorkspaceEdit,
};
pub use memory::{
    ArchitecturePattern, DiscoveredRelationship, ImportantSymbol, IntelligenceMemory,
    IntelligenceMemoryTrait, ProjectIntelligence, ProjectStructure,
};
pub use parser::{
    create_parser, create_parser_trait, parse_file, parse_source, CodeParserTrait, ParseResult,
    ParsedSymbol, ParserSymbolKind, TreeSitterParser,
};
pub use reasoning::{AgentReasoningEngine, ReasoningEngineTrait, ReasoningResult, ReasoningStep};
pub use search::{MatchType, SearchResult, SemanticSearch, SemanticSearchTrait};

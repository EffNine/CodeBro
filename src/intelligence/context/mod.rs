#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Context Builder — Intelligence context assembly abstraction layer.
/// Provides a trait-based interface for building contextual information
/// from the symbol index and dependency graph.
pub mod builder;

use anyhow::Result;

pub use builder::{CodeSnippet, IntelligenceContext, IntelligentContextBuilder};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for building intelligence contexts.
///
/// Implementations assemble relevant symbols, files, and code snippets
/// for a given query or modification target.
///
/// Note: SQLite connections are not thread-safe; this trait is Send-only.
pub trait ContextBuilderTrait: Send {
    /// Build context for a general query.
    fn build_context(&self, query: &str) -> Result<IntelligenceContext>;

    /// Build context optimized for code modification analysis.
    /// Expands the dependency graph around the target symbol.
    fn build_context_for_modification(&self, target_symbol: &str) -> Result<IntelligenceContext>;

    /// Get symbols related to a given symbol name.
    fn get_related_symbols(
        &self,
        symbol_name: &str,
    ) -> Result<Vec<crate::intelligence::search::SearchResult>>;

    /// Get file-level dependencies for a symbol.
    fn get_symbol_dependencies(&self, symbol_name: &str) -> Result<Vec<String>>;
}

impl ContextBuilderTrait for IntelligentContextBuilder {
    fn build_context(&self, query: &str) -> Result<IntelligenceContext> {
        IntelligentContextBuilder::build_context(self, query)
    }

    fn build_context_for_modification(&self, target_symbol: &str) -> Result<IntelligenceContext> {
        IntelligentContextBuilder::build_context_for_modification(self, target_symbol)
    }

    fn get_related_symbols(
        &self,
        symbol_name: &str,
    ) -> Result<Vec<crate::intelligence::search::SearchResult>> {
        IntelligentContextBuilder::get_related_symbols(self, symbol_name)
    }

    fn get_symbol_dependencies(&self, symbol_name: &str) -> Result<Vec<String>> {
        IntelligentContextBuilder::get_symbol_dependencies(self, symbol_name)
    }
}

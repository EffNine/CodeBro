#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Semantic Search — Code symbol search abstraction layer.
/// Provides a trait-based interface for semantic search over indexed symbols.
pub mod semantic;

use anyhow::Result;

pub use semantic::{MatchType, SearchResult, SemanticSearch};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for semantic search over indexed symbols.
///
/// Implementations provide keyword-based or embedding-based search
/// over the symbol database.
///
/// Note: SQLite connections are not thread-safe; this trait is Send-only.
pub trait SemanticSearchTrait: Send {
    fn new(indexer: crate::intelligence::index::CodeIndexer) -> Self;

    fn search(&self, query: &str) -> Result<Vec<SearchResult>>;
    fn search_by_question(&self, question: &str) -> Result<Vec<SearchResult>>;
    fn find_symbol(&self, name: &str)
        -> Result<Option<crate::intelligence::index::symbol::Symbol>>;
    fn find_symbols_by_file(
        &self,
        file: &str,
    ) -> Result<Vec<crate::intelligence::index::symbol::Symbol>>;
    fn find_symbols_by_kind(
        &self,
        kind: &str,
    ) -> Result<Vec<crate::intelligence::index::symbol::Symbol>>;
    fn find_symbols_by_language(
        &self,
        language: &str,
    ) -> Result<Vec<crate::intelligence::index::symbol::Symbol>>;
    fn find_related(&self, symbol_name: &str) -> Result<Vec<SearchResult>>;
}

impl SemanticSearchTrait for SemanticSearch {
    fn new(indexer: crate::intelligence::index::CodeIndexer) -> Self {
        SemanticSearch::new(indexer)
    }

    fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        SemanticSearch::search(self, query)
    }

    fn search_by_question(&self, question: &str) -> Result<Vec<SearchResult>> {
        SemanticSearch::search_by_question(self, question)
    }

    fn find_symbol(
        &self,
        name: &str,
    ) -> Result<Option<crate::intelligence::index::symbol::Symbol>> {
        SemanticSearch::find_symbol(self, name)
    }

    fn find_symbols_by_file(
        &self,
        file: &str,
    ) -> Result<Vec<crate::intelligence::index::symbol::Symbol>> {
        SemanticSearch::find_symbols_by_file(self, file)
    }

    fn find_symbols_by_kind(
        &self,
        kind: &str,
    ) -> Result<Vec<crate::intelligence::index::symbol::Symbol>> {
        SemanticSearch::find_symbols_by_kind(self, kind)
    }

    fn find_symbols_by_language(
        &self,
        language: &str,
    ) -> Result<Vec<crate::intelligence::index::symbol::Symbol>> {
        SemanticSearch::find_symbols_by_language(self, language)
    }

    fn find_related(&self, symbol_name: &str) -> Result<Vec<SearchResult>> {
        SemanticSearch::find_related(self, symbol_name)
    }
}

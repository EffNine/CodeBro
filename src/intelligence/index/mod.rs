#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Index Platform - Symbol indexing and storage abstraction layer.
//!
//! Provides a trait-based interface for symbol storage and indexing.

pub mod database;
pub mod indexer;
pub mod symbol;

use std::path::{Path, PathBuf};

use anyhow::Result;

pub use database::SymbolDatabase;
pub use indexer::CodeIndexer;
pub use symbol::{FileInfo, Symbol, SymbolKind, SymbolRelationship};

// Update the indexer to use the re-exported type
pub use crate::intelligence::parser::TreeSitterParser as CodeParser;

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for symbol database backends.
///
/// Implementations provide persistent storage for symbols and relationships.
/// Note: SQLite connections are not thread-safe; this trait is Send-only.
pub trait SymbolDatabaseTrait: Send {
    fn insert_symbol(&self, symbol: &Symbol) -> Result<i64>;
    fn insert_symbols(&self, symbols: &[Symbol]) -> Result<()>;
    fn get_symbol_by_name(&self, name: &str) -> Result<Option<Symbol>>;
    fn get_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>>;
    fn get_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>>;
    fn get_symbols_by_language(&self, language: &str) -> Result<Vec<Symbol>>;
    fn search_symbols(&self, query: &str) -> Result<Vec<Symbol>>;
    fn get_all_symbols(&self) -> Result<Vec<Symbol>>;
    fn get_symbol_count(&self) -> Result<u32>;
    fn delete_symbols_by_file(&self, file: &str) -> Result<()>;
    fn delete_all_symbols(&self) -> Result<()>;

    // Relationship queries
    fn insert_relationship(&self, relationship: &SymbolRelationship) -> Result<()>;
    fn get_relationships_for_symbol(&self, symbol_name: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependencies_for_file(&self, file: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependents_of_file(&self, file: &str) -> Result<Vec<SymbolRelationship>>;
}

impl SymbolDatabaseTrait for SymbolDatabase {
    fn insert_symbol(&self, symbol: &Symbol) -> Result<i64> {
        SymbolDatabase::insert_symbol(self, symbol)
    }

    fn insert_symbols(&self, symbols: &[Symbol]) -> Result<()> {
        SymbolDatabase::insert_symbols(self, symbols)
    }

    fn get_symbol_by_name(&self, name: &str) -> Result<Option<Symbol>> {
        SymbolDatabase::get_symbol_by_name(self, name)
    }

    fn get_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>> {
        SymbolDatabase::get_symbols_by_file(self, file)
    }

    fn get_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>> {
        SymbolDatabase::get_symbols_by_kind(self, kind)
    }

    fn get_symbols_by_language(&self, language: &str) -> Result<Vec<Symbol>> {
        SymbolDatabase::get_symbols_by_language(self, language)
    }

    fn search_symbols(&self, query: &str) -> Result<Vec<Symbol>> {
        SymbolDatabase::search_symbols(self, query)
    }

    fn get_all_symbols(&self) -> Result<Vec<Symbol>> {
        SymbolDatabase::get_all_symbols(self)
    }

    fn get_symbol_count(&self) -> Result<u32> {
        SymbolDatabase::get_symbol_count(self)
    }

    fn delete_symbols_by_file(&self, file: &str) -> Result<()> {
        SymbolDatabase::delete_symbols_by_file(self, file)
    }

    fn delete_all_symbols(&self) -> Result<()> {
        SymbolDatabase::delete_all_symbols(self)
    }

    fn insert_relationship(&self, relationship: &SymbolRelationship) -> Result<()> {
        SymbolDatabase::insert_relationship(self, relationship)
    }

    fn get_relationships_for_symbol(&self, symbol_name: &str) -> Result<Vec<SymbolRelationship>> {
        SymbolDatabase::get_relationships_for_symbol(self, symbol_name)
    }

    fn get_dependencies_for_file(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        SymbolDatabase::get_dependencies_for_file(self, file)
    }

    fn get_dependents_of_file(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        SymbolDatabase::get_dependents_of_file(self, file)
    }
}

/// Trait for code indexers.
///
/// Implementations provide file/directory indexing capabilities.
pub trait CodeIndexerTrait: Send {
    // File-level indexing
    fn index_file(&mut self, path: &Path, source: &str) -> Result<Vec<Symbol>>;
    fn incremental_update(&mut self, path: &Path, source: &str) -> Result<Vec<Symbol>>;
    fn remove_file(&mut self, path: &Path) -> Result<()>;

    // Directory-level indexing
    fn index_directory(&mut self, root: &Path) -> Result<Vec<Symbol>>;

    // Query interface
    fn get_symbols(&self) -> Result<Vec<Symbol>>;
    fn find_symbol(&self, name: &str) -> Result<Option<Symbol>>;
    fn find_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>>;
    fn find_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>>;
    fn find_symbols_by_language(&self, lang: &str) -> Result<Vec<Symbol>>;
    fn search(&self, query: &str) -> Result<Vec<Symbol>>;
    fn get_symbol_count(&self) -> Result<u32>;

    // Relationship queries
    fn get_relationships(&self, symbol_name: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependencies(&self, file: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependents(&self, file: &str) -> Result<Vec<SymbolRelationship>>;

    // Maintenance
    fn clear(&mut self) -> Result<()>;
    fn get_indexed_files(&self) -> Vec<String>;
}

impl CodeIndexerTrait for CodeIndexer {
    fn index_file(&mut self, path: &Path, source: &str) -> Result<Vec<Symbol>> {
        CodeIndexer::index_file(self, path, source)
    }

    fn incremental_update(&mut self, path: &Path, source: &str) -> Result<Vec<Symbol>> {
        CodeIndexer::incremental_update(self, path, source)
    }

    fn remove_file(&mut self, path: &Path) -> Result<()> {
        CodeIndexer::remove_file(self, path)
    }

    fn index_directory(&mut self, root: &Path) -> Result<Vec<Symbol>> {
        CodeIndexer::index_directory(self, root)
    }

    fn get_symbols(&self) -> Result<Vec<Symbol>> {
        CodeIndexer::get_symbols(self)
    }

    fn find_symbol(&self, name: &str) -> Result<Option<Symbol>> {
        CodeIndexer::find_symbol(self, name)
    }

    fn find_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>> {
        CodeIndexer::find_symbols_by_file(self, file)
    }

    fn find_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>> {
        CodeIndexer::find_symbols_by_kind(self, kind)
    }

    fn find_symbols_by_language(&self, lang: &str) -> Result<Vec<Symbol>> {
        CodeIndexer::find_symbols_by_language(self, lang)
    }

    fn search(&self, query: &str) -> Result<Vec<Symbol>> {
        CodeIndexer::search(self, query)
    }

    fn get_symbol_count(&self) -> Result<u32> {
        CodeIndexer::get_symbol_count(self)
    }

    fn get_relationships(&self, symbol_name: &str) -> Result<Vec<SymbolRelationship>> {
        CodeIndexer::get_relationships(self, symbol_name)
    }

    fn get_dependencies(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        CodeIndexer::get_dependencies(self, file)
    }

    fn get_dependents(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        CodeIndexer::get_dependents(self, file)
    }

    fn clear(&mut self) -> Result<()> {
        CodeIndexer::clear(self)
    }

    fn get_indexed_files(&self) -> Vec<String> {
        CodeIndexer::list_indexed_files(self).unwrap_or_default()
    }
}

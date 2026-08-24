#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Dependency Graph — Code dependency graph abstraction layer.
/// Provides a trait-based interface for code dependency graphs.
pub mod dependency;

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;

pub use dependency::{DependencyGraph, DependencyNode};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for dependency graphs.
///
/// Implementations provide graph-based representation of code dependencies.
pub trait DependencyGraphTrait: Send + Sync {
    fn new() -> Self;

    /// Build a graph from an indexer's symbols and relationships.
    fn from_indexer(indexer: &crate::intelligence::index::CodeIndexer) -> Result<Self>
    where
        Self: Sized;

    fn add_node(&mut self, file: String);
    fn add_edge(&mut self, from_file: String, to_file: String);

    fn get_dependencies(&self, file: &str) -> Vec<String>;
    fn get_dependents(&self, file: &str) -> Vec<String>;
    fn get_transitive_dependencies(&self, file: &str) -> HashSet<String>;
    fn get_transitive_dependents(&self, file: &str) -> HashSet<String>;
    fn get_all_files(&self) -> Vec<String>;
    fn get_symbol_files(&self, symbol_name: &str) -> Vec<String>;
    fn find_path(&self, from: &str, to: &str) -> Option<Vec<String>>;

    fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()>;
    fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self>
    where
        Self: Sized;
}

impl DependencyGraphTrait for DependencyGraph {
    fn new() -> Self {
        DependencyGraph::new()
    }

    fn from_indexer(indexer: &crate::intelligence::index::CodeIndexer) -> Result<Self> {
        DependencyGraph::from_indexer(indexer)
    }

    fn add_node(&mut self, file: String) {
        DependencyGraph::add_node(self, file)
    }

    fn add_edge(&mut self, from_file: String, to_file: String) {
        DependencyGraph::add_edge(self, from_file, to_file)
    }

    fn get_dependencies(&self, file: &str) -> Vec<String> {
        DependencyGraph::get_dependencies(self, file)
    }

    fn get_dependents(&self, file: &str) -> Vec<String> {
        DependencyGraph::get_dependents(self, file)
    }

    fn get_transitive_dependencies(&self, file: &str) -> HashSet<String> {
        DependencyGraph::get_transitive_dependencies(self, file)
    }

    fn get_transitive_dependents(&self, file: &str) -> HashSet<String> {
        DependencyGraph::get_transitive_dependents(self, file)
    }

    fn get_all_files(&self) -> Vec<String> {
        DependencyGraph::get_all_files(self)
    }

    fn get_symbol_files(&self, symbol_name: &str) -> Vec<String> {
        DependencyGraph::get_symbol_files(self, symbol_name)
    }

    fn find_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        DependencyGraph::find_path(self, from, to)
    }

    fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        DependencyGraph::save_to_file(self, path)
    }

    fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        DependencyGraph::load_from_file(path)
    }
}

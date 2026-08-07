#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Intelligence Memory — Project intelligence persistence abstraction layer.
/// Provides a trait-based interface for persisting and querying
/// project-level intelligence (patterns, conventions, important symbols).
pub mod intelligence;

use anyhow::Result;

pub use intelligence::{
    ArchitecturePattern, DiscoveredRelationship, ImportantSymbol, IntelligenceMemory,
    ProjectIntelligence, ProjectStructure,
};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for intelligence memory.
///
/// Implementations persist and retrieve project-level knowledge
/// such as important symbols, architecture patterns, and conventions.
pub trait IntelligenceMemoryTrait: Send + Sync {
    fn new() -> Result<Self>
    where
        Self: Sized;

    fn save(&self) -> Result<()>;

    // Recording
    fn record_symbol(&mut self, name: String, kind: String, file: String, reason: String);
    fn record_pattern(
        &mut self,
        name: String,
        description: String,
        files: Vec<String>,
        confidence: f32,
    );
    fn record_convention(&mut self, convention: String);
    fn record_relationship(
        &mut self,
        from_symbol: String,
        to_symbol: String,
        relationship_type: String,
        file: String,
    );

    // Querying
    fn get_important_symbols(&self) -> &[ImportantSymbol];
    fn get_architecture_patterns(&self) -> &[ArchitecturePattern];
    fn get_conventions(&self) -> &[String];
    fn get_relationships(&self) -> &[DiscoveredRelationship];
    fn get_project_structure(&self) -> Option<&ProjectStructure>;
    fn set_project_structure(&mut self, structure: ProjectStructure);

    // Analysis
    fn analyze_project(&mut self, indexer: &crate::intelligence::index::CodeIndexer) -> Result<()>;
}

impl IntelligenceMemoryTrait for IntelligenceMemory {
    fn new() -> Result<Self> {
        IntelligenceMemory::new()
    }

    fn save(&self) -> Result<()> {
        IntelligenceMemory::save(self)
    }

    fn record_symbol(&mut self, name: String, kind: String, file: String, reason: String) {
        IntelligenceMemory::record_symbol(self, name, kind, file, reason);
    }

    fn record_pattern(
        &mut self,
        name: String,
        description: String,
        files: Vec<String>,
        confidence: f32,
    ) {
        IntelligenceMemory::record_pattern(self, name, description, files, confidence);
    }

    fn record_convention(&mut self, convention: String) {
        IntelligenceMemory::record_convention(self, convention);
    }

    fn record_relationship(
        &mut self,
        from_symbol: String,
        to_symbol: String,
        relationship_type: String,
        file: String,
    ) {
        IntelligenceMemory::record_relationship(
            self,
            from_symbol,
            to_symbol,
            relationship_type,
            file,
        );
    }

    fn get_important_symbols(&self) -> &[ImportantSymbol] {
        IntelligenceMemory::get_important_symbols(self)
    }

    fn get_architecture_patterns(&self) -> &[ArchitecturePattern] {
        IntelligenceMemory::get_architecture_patterns(self)
    }

    fn get_conventions(&self) -> &[String] {
        IntelligenceMemory::get_conventions(self)
    }

    fn get_relationships(&self) -> &[DiscoveredRelationship] {
        IntelligenceMemory::get_relationships(self)
    }

    fn get_project_structure(&self) -> Option<&ProjectStructure> {
        IntelligenceMemory::get_project_structure(self)
    }

    fn set_project_structure(&mut self, structure: ProjectStructure) {
        IntelligenceMemory::set_project_structure(self, structure);
    }

    fn analyze_project(&mut self, indexer: &crate::intelligence::index::CodeIndexer) -> Result<()> {
        IntelligenceMemory::analyze_project(self, indexer)
    }
}

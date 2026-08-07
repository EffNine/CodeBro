#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectIntelligence {
    pub important_symbols: Vec<ImportantSymbol>,
    pub architecture_patterns: Vec<ArchitecturePattern>,
    pub conventions: Vec<String>,
    pub discovered_relationships: Vec<DiscoveredRelationship>,
    pub key_files: Vec<String>,
    pub project_structure: Option<ProjectStructure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportantSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub reason: String,
    pub last_referenced: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitecturePattern {
    pub name: String,
    pub description: String,
    pub files_involved: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredRelationship {
    pub from_symbol: String,
    pub to_symbol: String,
    pub relationship_type: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectStructure {
    pub main_modules: Vec<String>,
    pub layers: Vec<String>,
    pub entry_points: Vec<String>,
    pub public_api: Vec<String>,
}

pub struct IntelligenceMemory {
    project_memory: ProjectIntelligence,
    memory_path: PathBuf,
}

impl IntelligenceMemory {
    pub fn new() -> Result<Self> {
        Self::new_with_path(&Config::config_dir().join("project_memory.json"))
    }

    pub fn new_with_path(memory_path: &std::path::Path) -> Result<Self> {
        let project_memory = if memory_path.exists() {
            let content = fs::read_to_string(memory_path).with_context(|| {
                format!("Failed to read project memory file: {:?}", memory_path)
            })?;
            serde_json::from_str(&content).with_context(|| {
                format!("Failed to parse project memory file: {:?}", memory_path)
            })?
        } else {
            ProjectIntelligence::default()
        };

        Ok(IntelligenceMemory {
            project_memory,
            memory_path: memory_path.to_path_buf(),
        })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.memory_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(&self.project_memory)?;
        fs::write(&self.memory_path, content)?;
        Ok(())
    }

    pub fn record_symbol(&mut self, name: String, kind: String, file: String, reason: String) {
        let symbol = ImportantSymbol {
            name,
            kind,
            file,
            reason,
            last_referenced: Some(chrono::Local::now().to_rfc3339()),
        };

        if !self
            .project_memory
            .important_symbols
            .iter()
            .any(|s| s.name == symbol.name)
        {
            self.project_memory.important_symbols.push(symbol);
        }
    }

    pub fn record_pattern(
        &mut self,
        name: String,
        description: String,
        files: Vec<String>,
        confidence: f32,
    ) {
        let pattern = ArchitecturePattern {
            name,
            description,
            files_involved: files,
            confidence,
        };
        self.project_memory.architecture_patterns.push(pattern);
    }

    pub fn record_convention(&mut self, convention: String) {
        if !self.project_memory.conventions.contains(&convention) {
            self.project_memory.conventions.push(convention);
        }
    }

    pub fn record_relationship(
        &mut self,
        from_symbol: String,
        to_symbol: String,
        relationship_type: String,
        file: String,
    ) {
        let relationship = DiscoveredRelationship {
            from_symbol,
            to_symbol,
            relationship_type,
            file,
        };
        self.project_memory
            .discovered_relationships
            .push(relationship);
    }

    pub fn get_important_symbols(&self) -> &[ImportantSymbol] {
        &self.project_memory.important_symbols
    }

    pub fn get_architecture_patterns(&self) -> &[ArchitecturePattern] {
        &self.project_memory.architecture_patterns
    }

    pub fn get_conventions(&self) -> &[String] {
        &self.project_memory.conventions
    }

    pub fn get_relationships(&self) -> &[DiscoveredRelationship] {
        &self.project_memory.discovered_relationships
    }

    pub fn get_project_structure(&self) -> Option<&ProjectStructure> {
        self.project_memory.project_structure.as_ref()
    }

    pub fn set_project_structure(&mut self, structure: ProjectStructure) {
        self.project_memory.project_structure = Some(structure);
    }

    pub fn analyze_project(
        &mut self,
        indexer: &crate::intelligence::index::CodeIndexer,
    ) -> Result<()> {
        let symbols = indexer.get_symbols()?;

        for symbol in &symbols {
            let reason = match &symbol.kind {
                crate::intelligence::index::symbol::SymbolKind::Trait => {
                    "Core trait definition".to_string()
                }
                crate::intelligence::index::symbol::SymbolKind::Interface => {
                    "Core interface definition".to_string()
                }
                crate::intelligence::index::symbol::SymbolKind::Struct => {
                    "Core data structure".to_string()
                }
                crate::intelligence::index::symbol::SymbolKind::Enum => {
                    "Core enum definition".to_string()
                }
                crate::intelligence::index::symbol::SymbolKind::Function => {
                    "Key function".to_string()
                }
                _ => "Important symbol".to_string(),
            };

            self.record_symbol(
                symbol.name.clone(),
                format!("{}", symbol.kind),
                symbol.file.clone(),
                reason,
            );
        }

        let files: Vec<String> = symbols.iter().map(|s| s.file.clone()).collect();
        let unique_files: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            files
                .into_iter()
                .filter(|f| seen.insert(f.clone()))
                .collect()
        };

        let main_modules: Vec<String> = unique_files
            .iter()
            .filter(|f| {
                f.contains("mod.rs")
                    || f.contains("lib.rs")
                    || f.contains("main.rs")
                    || f.contains("config")
                    || f.contains("types")
                    || f.contains("models")
            })
            .cloned()
            .collect();

        let structure = ProjectStructure {
            main_modules,
            layers: vec![
                "core".to_string(),
                "services".to_string(),
                "models".to_string(),
            ],
            entry_points: unique_files
                .iter()
                .filter(|f| f.ends_with("main.rs") || f.ends_with("lib.rs"))
                .cloned()
                .collect(),
            public_api: symbols
                .iter()
                .filter(|s| s.visibility.as_deref() == Some("public"))
                .map(|s| s.name.clone())
                .collect(),
        };

        self.set_project_structure(structure);

        self.save()?;

        Ok(())
    }
}

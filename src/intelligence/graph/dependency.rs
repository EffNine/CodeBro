#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::intelligence::index::symbol::SymbolRelationship;
use crate::intelligence::index::CodeIndexer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyNode {
    pub file: String,
    pub symbols: Vec<String>,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub nodes: HashMap<String, DependencyNode>,
    pub edges: Vec<SymbolRelationship>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        DependencyGraph {
            nodes: HashMap::new(),
            edges: Vec::new(),
        }
    }

    pub fn from_indexer(indexer: &CodeIndexer) -> Result<Self> {
        let mut graph = DependencyGraph::new();

        let symbols = indexer.get_symbols()?;
        let all_files: HashSet<String> = symbols.iter().map(|s| s.file.clone()).collect();

        for symbol in &symbols {
            let node = graph
                .nodes
                .entry(symbol.file.clone())
                .or_insert_with(|| DependencyNode {
                    file: symbol.file.clone(),
                    symbols: Vec::new(),
                    dependencies: Vec::new(),
                    dependents: Vec::new(),
                });
            node.symbols.push(symbol.name.clone());
        }

        let all_files_vec: Vec<String> = all_files.iter().cloned().collect();
        let mut dependent_updates: Vec<(String, String)> = Vec::new();

        for file in &all_files_vec {
            let relationships = indexer.get_dependencies(file)?;
            let node = graph
                .nodes
                .entry(file.clone())
                .or_insert_with(|| DependencyNode {
                    file: file.clone(),
                    symbols: Vec::new(),
                    dependencies: Vec::new(),
                    dependents: Vec::new(),
                });

            for rel in &relationships {
                if !rel.to_file.is_empty() && rel.to_file != *file {
                    node.dependencies.push(rel.to_file.clone());
                    graph.edges.push(rel.clone());

                    dependent_updates.push((rel.to_file.clone(), file.clone()));
                }
            }
        }

        for (target_file, dependent_file) in dependent_updates {
            if let Some(target_node) = graph.nodes.get_mut(&target_file) {
                if !target_node.dependents.contains(&dependent_file) {
                    target_node.dependents.push(dependent_file);
                }
            }
        }

        Ok(graph)
    }

    pub fn add_node(&mut self, file: String) {
        self.nodes
            .entry(file.clone())
            .or_insert_with(|| DependencyNode {
                file,
                symbols: Vec::new(),
                dependencies: Vec::new(),
                dependents: Vec::new(),
            });
    }

    pub fn add_edge(&mut self, from_file: String, to_file: String) {
        if let Some(node) = self.nodes.get_mut(&from_file) {
            if !node.dependencies.contains(&to_file) {
                node.dependencies.push(to_file.clone());
            }
        }

        if let Some(node) = self.nodes.get_mut(&to_file) {
            if !node.dependents.contains(&from_file) {
                node.dependents.push(from_file.clone());
            }
        }

        self.edges.push(SymbolRelationship {
            from_symbol: String::new(),
            from_file: from_file.clone(),
            to_symbol: String::new(),
            to_file: to_file.clone(),
            relationship_type: "depends_on".to_string(),
        });
    }

    pub fn get_dependencies(&self, file: &str) -> Vec<String> {
        self.nodes
            .get(file)
            .map(|n| n.dependencies.clone())
            .unwrap_or_default()
    }

    pub fn get_dependents(&self, file: &str) -> Vec<String> {
        self.nodes
            .get(file)
            .map(|n| n.dependents.clone())
            .unwrap_or_default()
    }

    pub fn get_transitive_dependencies(&self, file: &str) -> HashSet<String> {
        let mut visited = HashSet::new();
        self.collect_dependencies(file, &mut visited);
        visited.remove(file);
        visited
    }

    fn collect_dependencies(&self, file: &str, visited: &mut HashSet<String>) {
        if visited.contains(file) {
            return;
        }
        visited.insert(file.to_string());

        if let Some(node) = self.nodes.get(file) {
            for dep in &node.dependencies {
                self.collect_dependencies(dep, visited);
            }
        }
    }

    pub fn get_transitive_dependents(&self, file: &str) -> HashSet<String> {
        let mut visited = HashSet::new();
        self.collect_dependents(file, &mut visited);
        visited.remove(file);
        visited
    }

    fn collect_dependents(&self, file: &str, visited: &mut HashSet<String>) {
        if visited.contains(file) {
            return;
        }
        visited.insert(file.to_string());

        if let Some(node) = self.nodes.get(file) {
            for dep in &node.dependents {
                self.collect_dependents(dep, visited);
            }
        }
    }

    pub fn get_all_files(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    pub fn get_symbol_files(&self, symbol_name: &str) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.symbols.iter().any(|s| s.contains(symbol_name)))
            .map(|(_, node)| node.file.clone())
            .collect()
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize dependency graph")?;
        std::fs::write(path, json).context("Failed to write dependency graph file")?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content =
            std::fs::read_to_string(&path).context("Failed to read dependency graph file")?;
        let graph: DependencyGraph =
            serde_json::from_str(&content).context("Failed to parse dependency graph JSON")?;
        Ok(graph)
    }

    pub fn find_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        if from == to {
            return Some(vec![from.to_string()]);
        }

        let mut visited = HashSet::new();
        let mut queue: Vec<(String, Vec<String>)> =
            vec![(from.to_string(), vec![from.to_string()])];

        while let Some((current, path)) = queue.pop() {
            if current == to {
                return Some(path);
            }

            if visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());

            if let Some(node) = self.nodes.get(&current) {
                for dep in &node.dependencies {
                    if !visited.contains(dep) {
                        let mut new_path = path.clone();
                        new_path.push(dep.clone());
                        queue.push((dep.clone(), new_path));
                    }
                }
            }
        }

        None
    }
}

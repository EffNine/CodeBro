#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;

use crate::intelligence::graph::DependencyGraph;
use crate::intelligence::index::CodeIndexer;

use crate::intelligence::search::SearchResult;
use crate::intelligence::search::SemanticSearch;

#[derive(Debug, Clone)]
pub struct IntelligenceContext {
    pub query: String,
    pub relevant_symbols: Vec<SearchResult>,
    pub related_files: Vec<String>,
    pub dependencies: Vec<String>,
    pub imports: Vec<String>,
    pub code_snippets: Vec<CodeSnippet>,
    pub total_symbols_found: usize,
}

#[derive(Debug, Clone)]
pub struct CodeSnippet {
    pub file: String,
    pub content: String,
    pub symbol_name: Option<String>,
    pub relevance: f32,
}

#[derive(Clone)]
pub struct IntelligentContextBuilder {
    indexer: CodeIndexer,
    search: SemanticSearch,
    dependency_graph: Option<DependencyGraph>,
    max_symbols: usize,
    max_files: usize,
    max_snippet_length: usize,
}

impl IntelligentContextBuilder {
    pub fn new(indexer: CodeIndexer) -> Self {
        let search = SemanticSearch::new(indexer.clone());

        IntelligentContextBuilder {
            indexer,
            search,
            dependency_graph: None,
            max_symbols: 20,
            max_files: 10,
            max_snippet_length: 500,
        }
    }

    pub fn with_dependency_graph(mut self, graph: DependencyGraph) -> Self {
        self.dependency_graph = Some(graph);
        self
    }

    pub fn with_max_symbols(mut self, max: usize) -> Self {
        self.max_symbols = max;
        self
    }

    pub fn with_max_files(mut self, max: usize) -> Self {
        self.max_files = max;
        self
    }

    pub fn with_max_snippet_length(mut self, max: usize) -> Self {
        self.max_snippet_length = max;
        self
    }

    pub fn build_context(&self, query: &str) -> Result<IntelligenceContext> {
        let search_results = self.search.search_by_question(query)?;
        let relevant_symbols: Vec<SearchResult> =
            search_results.into_iter().take(self.max_symbols).collect();

        let mut related_files: Vec<String> = relevant_symbols
            .iter()
            .map(|r| r.symbol.file.clone())
            .collect();
        related_files.dedup();
        related_files.truncate(self.max_files);

        let mut dependencies = Vec::new();
        let imports = Vec::new();

        if let Some(ref graph) = self.dependency_graph {
            for file in &related_files {
                let deps = graph.get_dependencies(file);
                dependencies.extend(deps);

                let dependents = graph.get_dependents(file);
                dependencies.extend(dependents);
            }
            dependencies.dedup();
        }

        let mut code_snippets = Vec::new();
        for result in &relevant_symbols {
            if let Ok(_symbols) = self.indexer.find_symbols_by_file(&result.symbol.file) {
                let content = std::fs::read_to_string(&result.symbol.file).unwrap_or_default();

                let snippet = self.extract_relevant_snippet(
                    &content,
                    result.symbol.line_start,
                    result.symbol.line_end,
                );

                code_snippets.push(CodeSnippet {
                    file: result.symbol.file.clone(),
                    content: snippet,
                    symbol_name: Some(result.symbol.name.clone()),
                    relevance: result.score,
                });
            }
        }

        code_snippets.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        code_snippets.truncate(self.max_files);

        let total_symbols_found = relevant_symbols.len();

        Ok(IntelligenceContext {
            query: query.to_string(),
            relevant_symbols,
            related_files,
            dependencies,
            imports,
            code_snippets,
            total_symbols_found,
        })
    }

    pub fn build_context_for_modification(
        &self,
        target_symbol: &str,
    ) -> Result<IntelligenceContext> {
        let mut context = self.build_context(target_symbol)?;

        if let Some(ref graph) = self.dependency_graph {
            let related_files = graph.get_transitive_dependencies(
                &context.related_files.first().cloned().unwrap_or_default(),
            );
            for file in related_files {
                if !context.related_files.contains(&file) {
                    context.related_files.push(file);
                }
            }

            let dependents = graph.get_transitive_dependents(
                &context.related_files.first().cloned().unwrap_or_default(),
            );
            for file in dependents {
                if !context.related_files.contains(&file) {
                    context.related_files.push(file);
                }
            }
        }

        context.related_files.truncate(self.max_files);

        Ok(context)
    }

    fn extract_relevant_snippet(&self, content: &str, line_start: u32, line_end: u32) -> String {
        let lines: Vec<&str> = content.lines().collect();

        let start = if line_start > 3 {
            (line_start - 3) as usize
        } else {
            0
        };
        let end = (line_end as usize).min(lines.len());

        let mut snippet_lines = Vec::new();
        for i in start..end {
            snippet_lines.push(lines[i]);
        }

        let snippet = snippet_lines.join("\n");

        if snippet.len() > self.max_snippet_length {
            snippet[..self.max_snippet_length].to_string()
        } else {
            snippet
        }
    }

    pub fn get_related_symbols(&self, symbol_name: &str) -> Result<Vec<SearchResult>> {
        self.search.find_related(symbol_name)
    }

    pub fn get_symbol_dependencies(&self, symbol_name: &str) -> Result<Vec<String>> {
        let mut deps = Vec::new();

        if let Ok(Some(symbol)) = self.indexer.find_symbol(symbol_name) {
            if let Some(ref graph) = self.dependency_graph {
                let file_deps = graph.get_dependencies(&symbol.file);
                deps.extend(file_deps);
            }
        }

        deps.dedup();
        Ok(deps)
    }
}

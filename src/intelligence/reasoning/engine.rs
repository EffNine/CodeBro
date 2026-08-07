#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};

use std::sync::Arc;

use crate::intelligence::context::{IntelligenceContext, IntelligentContextBuilder};
use crate::intelligence::index::symbol::SymbolKind;
use crate::intelligence::index::CodeIndexer;
use crate::intelligence::search::{SearchResult, SemanticSearch};

#[derive(Debug, Clone)]
pub struct ReasoningStep {
    pub step_number: u32,
    pub action: String,
    pub reasoning: String,
    pub symbols_found: Vec<String>,
    pub files_inspected: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct ReasoningResult {
    pub steps: Vec<ReasoningStep>,
    pub summary: String,
    pub plan: Vec<String>,
    pub relevant_context: IntelligenceContext,
    pub confidence: f32,
}

#[derive(Clone)]
pub struct AgentReasoningEngine {
    indexer: Arc<CodeIndexer>,
    search: SemanticSearch,
    context_builder: IntelligentContextBuilder,
}

impl AgentReasoningEngine {
    pub fn new(indexer: CodeIndexer) -> Self {
        let indexer = Arc::new(indexer);
        let search = SemanticSearch::new((*indexer).clone());
        let context_builder = IntelligentContextBuilder::new((*indexer).clone());

        AgentReasoningEngine {
            indexer,
            search,
            context_builder,
        }
    }

    pub fn indexer(&self) -> Arc<CodeIndexer> {
        self.indexer.clone()
    }

    pub fn analyze_before_modification(&self, user_request: &str) -> Result<ReasoningResult> {
        let mut steps = Vec::new();

        let step1 = ReasoningStep {
            step_number: 1,
            action: "Semantic Search".to_string(),
            reasoning: "Searching the codebase for relevant symbols related to the user's request"
                .to_string(),
            symbols_found: Vec::new(),
            files_inspected: Vec::new(),
            confidence: 0.0,
        };
        steps.push(step1);

        let search_results = self
            .search
            .search_by_question(user_request)
            .context("Failed to perform semantic search")?;

        let symbols_found: Vec<String> = search_results
            .iter()
            .take(10)
            .map(|r| format!("{} ({})", r.symbol.name, r.symbol.kind))
            .collect();

        let files_inspected: Vec<String> = search_results
            .iter()
            .take(10)
            .map(|r| r.symbol.file.clone())
            .collect();

        let confidence = if search_results.is_empty() {
            0.3
        } else {
            let avg_score: f32 =
                search_results.iter().map(|r| r.score).sum::<f32>() / search_results.len() as f32;
            avg_score.min(1.0)
        };

        let step2 = ReasoningStep {
            step_number: 2,
            action: "Symbol Lookup".to_string(),
            reasoning: format!(
                "Found {} relevant symbols in the codebase",
                search_results.len()
            ),
            symbols_found: symbols_found.clone(),
            files_inspected: files_inspected.clone(),
            confidence,
        };
        steps.push(step2);

        let step3 = ReasoningStep {
            step_number: 3,
            action: "Dependency Analysis".to_string(),
            reasoning: "Analyzing dependencies and relationships between symbols".to_string(),
            symbols_found: Vec::new(),
            files_inspected: Vec::new(),
            confidence: 0.0,
        };
        steps.push(step3);

        let context = self
            .context_builder
            .build_context(user_request)
            .context("Failed to build intelligence context")?;

        let related_symbols: Vec<String> = context
            .relevant_symbols
            .iter()
            .map(|r| r.symbol.name.clone())
            .collect();

        let step4 = ReasoningStep {
            step_number: 4,
            action: "Context Assembly".to_string(),
            reasoning: format!(
                "Assembled context with {} relevant symbols and {} related files",
                related_symbols.len(),
                context.related_files.len()
            ),
            symbols_found: related_symbols,
            files_inspected: context.related_files.clone(),
            confidence,
        };
        steps.push(step4);

        let plan = self.generate_plan(user_request, &search_results, &context)?;

        let overall_confidence = if search_results.is_empty() {
            0.4
        } else {
            confidence.max(0.5)
        };

        let summary = format!(
            "Analyzed '{}': found {} relevant symbols across {} files. Plan involves {} steps.",
            user_request,
            search_results.len(),
            context.related_files.len(),
            plan.len()
        );

        Ok(ReasoningResult {
            steps,
            summary,
            plan,
            relevant_context: context,
            confidence: overall_confidence,
        })
    }

    pub fn analyze_for_code_understanding(&self, file_path: &str) -> Result<ReasoningResult> {
        let mut steps = Vec::new();

        let step1 = ReasoningStep {
            step_number: 1,
            action: "File Analysis".to_string(),
            reasoning: format!("Analyzing file {} for symbols and structure", file_path),
            symbols_found: Vec::new(),
            files_inspected: vec![file_path.to_string()],
            confidence: 0.0,
        };
        steps.push(step1);

        let symbols = self
            .indexer
            .find_symbols_by_file(file_path)
            .context("Failed to find symbols in file")?;

        let symbols_found: Vec<String> = symbols
            .iter()
            .map(|s| format!("{} ({})", s.name, s.kind))
            .collect();

        let step2 = ReasoningStep {
            step_number: 2,
            action: "Dependency Resolution".to_string(),
            reasoning: "Resolving dependencies for the analyzed file".to_string(),
            symbols_found: symbols_found.clone(),
            files_inspected: vec![file_path.to_string()],
            confidence: 0.8,
        };
        steps.push(step2);

        let context = self
            .context_builder
            .build_context(file_path)
            .context("Failed to build context for file")?;

        let plan = vec![
            format!("Understand {} symbols in {}", symbols.len(), file_path),
            "Review dependencies and relationships".to_string(),
            "Check for existing patterns and conventions".to_string(),
        ];

        Ok(ReasoningResult {
            steps,
            summary: format!(
                "Analyzed {}: found {} symbols with {} dependencies",
                file_path,
                symbols.len(),
                context.dependencies.len()
            ),
            plan,
            relevant_context: context,
            confidence: 0.85,
        })
    }

    pub fn find_existing_patterns(&self, pattern_name: &str) -> Result<Vec<String>> {
        let results = self.search.search(pattern_name)?;
        let patterns: Vec<String> = results
            .iter()
            .filter(|r| {
                matches!(
                    r.symbol.kind,
                    SymbolKind::Function
                        | SymbolKind::Class
                        | SymbolKind::Struct
                        | SymbolKind::Method
                )
            })
            .map(|r| {
                format!(
                    "{} in {} (lines {}-{})",
                    r.symbol.name, r.symbol.file, r.symbol.line_start, r.symbol.line_end
                )
            })
            .collect();

        Ok(patterns)
    }

    pub fn suggest_implementation_approach(&self, user_request: &str) -> Result<Vec<String>> {
        let mut suggestions = Vec::new();

        let search_results = self.search.search_by_question(user_request)?;

        if search_results.is_empty() {
            suggestions
                .push("No existing symbols found. Consider creating new abstractions.".to_string());
            suggestions.push("Start with a minimal implementation and iterate.".to_string());
        } else {
            let top_results: Vec<_> = search_results.iter().take(5).collect();

            let has_existing_interface = top_results
                .iter()
                .any(|r| matches!(r.symbol.kind, SymbolKind::Trait | SymbolKind::Interface));

            if has_existing_interface {
                suggestions.push(
                    "Extend existing interface/abstraction rather than creating new ones."
                        .to_string(),
                );
            }

            let has_existing_implementation = top_results.iter().any(|r| {
                matches!(
                    r.symbol.kind,
                    SymbolKind::Function | SymbolKind::Method | SymbolKind::Class
                )
            });

            if has_existing_implementation {
                suggestions
                    .push("Follow existing implementation patterns in the codebase.".to_string());
            }

            let has_config = top_results.iter().any(|r| {
                r.symbol.name.to_lowercase().contains("config")
                    || r.symbol.name.to_lowercase().contains("setting")
            });

            if has_config {
                suggestions
                    .push("Add configuration support following existing patterns.".to_string());
            }

            suggestions.push("Add tests for the new functionality.".to_string());
        }

        Ok(suggestions)
    }

    fn generate_plan(
        &self,
        _user_request: &str,
        search_results: &[SearchResult],
        _context: &IntelligenceContext,
    ) -> Result<Vec<String>> {
        let mut plan = Vec::new();

        if search_results.is_empty() {
            plan.push("Create new implementation based on the request".to_string());
            plan.push("Add necessary imports and dependencies".to_string());
            plan.push("Write tests for the new functionality".to_string());
            return Ok(plan);
        }

        let top_symbols: Vec<_> = search_results.iter().take(3).collect();

        let has_interface = top_symbols
            .iter()
            .any(|r| matches!(r.symbol.kind, SymbolKind::Trait | SymbolKind::Interface));

        if has_interface {
            plan.push("Extend existing interface/abstraction".to_string());
        }

        let has_existing_impl = top_symbols.iter().any(|r| {
            matches!(
                r.symbol.kind,
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Class
            )
        });

        if has_existing_impl {
            plan.push("Implement or modify existing symbols".to_string());
        }

        let has_config = top_symbols.iter().any(|r| {
            r.symbol.name.to_lowercase().contains("config")
                || r.symbol.name.to_lowercase().contains("setting")
        });

        if has_config {
            plan.push("Update configuration if needed".to_string());
        }

        plan.push("Add tests for the changes".to_string());

        Ok(plan)
    }
}

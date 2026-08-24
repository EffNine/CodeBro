#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;

use crate::intelligence::index::symbol::Symbol;
use crate::intelligence::index::CodeIndexer;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub symbol: Symbol,
    pub score: f32,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchType {
    ExactName,
    PartialName,
    SymbolType,
    FileRelevance,
    TextMatch,
    Dependency,
}

#[derive(Clone)]
pub struct SemanticSearch {
    indexer: CodeIndexer,
}

impl SemanticSearch {
    pub fn new(indexer: CodeIndexer) -> Self {
        SemanticSearch { indexer }
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        let all_symbols = self.indexer.get_symbols()?;

        let mut results: Vec<SearchResult> = Vec::new();

        for symbol in &all_symbols {
            let score = self.score_symbol(symbol, &query_lower, &query_terms);
            if score > 0.0 {
                let match_type = self.determine_match_type(symbol, &query_lower, &query_terms);
                results.push(SearchResult {
                    symbol: symbol.clone(),
                    score,
                    match_type,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    pub fn find_symbol(&self, name: &str) -> Result<Option<Symbol>> {
        self.indexer.find_symbol(name)
    }

    pub fn find_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>> {
        self.indexer.find_symbols_by_file(file)
    }

    pub fn find_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>> {
        self.indexer.find_symbols_by_kind(kind)
    }

    pub fn find_symbols_by_language(&self, language: &str) -> Result<Vec<Symbol>> {
        self.indexer.find_symbols_by_language(language)
    }

    pub fn find_related(&self, symbol_name: &str) -> Result<Vec<SearchResult>> {
        let relationships = self.indexer.get_relationships(symbol_name)?;
        let mut results = Vec::new();

        for rel in &relationships {
            if let Ok(Some(symbol)) = self.indexer.find_symbol(&rel.to_symbol) {
                results.push(SearchResult {
                    symbol,
                    score: 0.8,
                    match_type: MatchType::Dependency,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    pub fn search_by_question(&self, question: &str) -> Result<Vec<SearchResult>> {
        let question_lower = question.to_lowercase();

        let mut results = self.search(&question_lower)?;

        let question_terms: Vec<&str> = question_lower.split_whitespace().collect();

        for result in &mut results {
            for term in &question_terms {
                if result.symbol.name.to_lowercase().contains(term) {
                    result.score += 0.3;
                }
                if let Some(ref sig) = result.symbol.signature {
                    if sig.to_lowercase().contains(term) {
                        result.score += 0.2;
                    }
                }
                if let Some(ref doc) = result.symbol.doc_comment {
                    if doc.to_lowercase().contains(term) {
                        result.score += 0.15;
                    }
                }
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    fn score_symbol(&self, symbol: &Symbol, query_lower: &str, query_terms: &[&str]) -> f32 {
        let mut score = 0.0;

        let name_lower = symbol.name.to_lowercase();

        for term in query_terms {
            if name_lower == *term {
                score += 3.0;
            } else if name_lower.starts_with(term) {
                score += 2.0;
            } else if name_lower.contains(term) {
                score += 1.5;
            }
        }

        if name_lower.contains(query_lower) {
            score += 2.0;
        }

        if let Some(ref sig) = symbol.signature {
            let sig_lower = sig.to_lowercase();
            if sig_lower.contains(query_lower) {
                score += 1.0;
            }
            for term in query_terms {
                if sig_lower.contains(term) {
                    score += 0.5;
                }
            }
        }

        if let Some(ref doc) = symbol.doc_comment {
            let doc_lower = doc.to_lowercase();
            if doc_lower.contains(query_lower) {
                score += 0.8;
            }
            for term in query_terms {
                if doc_lower.contains(term) {
                    score += 0.3;
                }
            }
        }

        let file_lower = symbol.file.to_lowercase();
        for term in query_terms {
            if file_lower.contains(term) {
                score += 0.5;
            }
        }

        score
    }

    fn determine_match_type(
        &self,
        symbol: &Symbol,
        query_lower: &str,
        query_terms: &[&str],
    ) -> MatchType {
        let name_lower = symbol.name.to_lowercase();

        for term in query_terms {
            if name_lower == *term {
                return MatchType::ExactName;
            }
        }

        if name_lower.contains(query_lower) {
            return MatchType::PartialName;
        }

        if let Some(ref sig) = symbol.signature {
            if sig.to_lowercase().contains(query_lower) {
                return MatchType::TextMatch;
            }
        }

        if let Some(ref doc) = symbol.doc_comment {
            if doc.to_lowercase().contains(query_lower) {
                return MatchType::TextMatch;
            }
        }

        MatchType::FileRelevance
    }
}

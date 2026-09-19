#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Parser Platform — Code parsing abstraction layer.
//!
//! Provides language-agnostic parsing through the `CodeParser` trait.
//! The default implementation uses tree-sitter.

pub mod languages;
pub mod tree_sitter;

use anyhow::Result;

// Re-export concrete types
pub use tree_sitter::{
    create_parser, parse_file, parse_source, CodeParser as TreeSitterParser, ParseCall,
    ParseImport, ParseResult, ParsedSymbol, SymbolKind, SymbolKind as ParserSymbolKind,
};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for code parsers.
///
/// Implementations must be able to parse source code into a structured
/// representation of symbols and imports.
pub trait CodeParserTrait: Send {
    /// Parse source code into symbols.
    fn parse(&mut self, source: &str, file_path: &str) -> Result<ParseResult>;

    /// Return the list of supported language names.
    fn supported_languages(&self) -> Vec<&str>;

    /// Return the name of the language this parser is configured for.
    fn language_name(&self) -> &str;
}

impl CodeParserTrait for TreeSitterParser {
    fn parse(&mut self, source: &str, file_path: &str) -> Result<ParseResult> {
        self.parse_source(source, file_path)
    }

    fn supported_languages(&self) -> Vec<&str> {
        languages::get_supported_languages()
    }

    fn language_name(&self) -> &str {
        self.language_name()
    }
}

/// Factory function to create a parser for a given language.
pub fn create_parser_trait(language: &str) -> Result<Box<dyn CodeParserTrait>> {
    let parser = TreeSitterParser::new(language)?;
    Ok(Box::new(parser))
}

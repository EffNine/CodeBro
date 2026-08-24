#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! LSP Abstraction — Language Server Protocol foundation types and interface.
/// Provides LSP-compatible types and a trait-based foundation for
/// future LSP server integration.
pub mod foundation;

use anyhow::Result;

pub use foundation::{
    create_lsp_foundation, DiagnosticSeverity, LspDiagnostic, LspFoundation, LspHover, LspLocation,
    LspParameterInformation, LspPosition, LspRange, LspSignatureInformation, LspSymbolInformation,
    LspTextDocumentIdentifier, LspTextDocumentItem, LspTextEdit, LspWorkspaceEdit,
    SymbolKind as LspSymbolKind,
};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for LSP foundation operations.
///
/// Implementations provide document management, symbol lookup,
/// and diagnostic capabilities following LSP protocol types.
pub trait LspFoundationTrait: Send + Sync {
    fn new() -> Self;

    // Document management
    fn open_document(&mut self, document: LspTextDocumentItem);
    fn close_document(&mut self, uri: &str);
    fn get_document(&self, uri: &str) -> Option<&LspTextDocumentItem>;
    fn update_document(&mut self, uri: &str, text: String, version: i32);
    fn get_text(&self, uri: &str) -> Option<String>;

    // Symbol management
    fn add_symbol(&mut self, symbol: LspSymbolInformation);
    fn get_symbols_for_file(&self, file: &str) -> Vec<LspSymbolInformation>;

    // Diagnostics
    fn add_diagnostic(&mut self, diagnostic: LspDiagnostic);
    fn get_diagnostics_for_file(&self, file: &str) -> Vec<LspDiagnostic>;

    // Navigation
    fn find_definition(&self, uri: &str, position: &LspPosition) -> Option<LspLocation>;
    fn find_references(&self, symbol_name: &str) -> Vec<LspLocation>;
    fn rename_symbol(
        &self,
        uri: &str,
        position: &LspPosition,
        new_name: &str,
    ) -> Option<LspWorkspaceEdit>;
}

impl LspFoundationTrait for LspFoundation {
    fn new() -> Self {
        LspFoundation::new()
    }

    fn open_document(&mut self, document: LspTextDocumentItem) {
        LspFoundation::open_document(self, document);
    }

    fn close_document(&mut self, uri: &str) {
        LspFoundation::close_document(self, uri);
    }

    fn get_document(&self, uri: &str) -> Option<&LspTextDocumentItem> {
        LspFoundation::get_document(self, uri)
    }

    fn update_document(&mut self, uri: &str, text: String, version: i32) {
        LspFoundation::update_document(self, uri, text, version);
    }

    fn get_text(&self, uri: &str) -> Option<String> {
        LspFoundation::get_text(self, uri)
    }

    fn add_symbol(&mut self, symbol: LspSymbolInformation) {
        LspFoundation::add_symbol(self, symbol);
    }

    fn get_symbols_for_file(&self, file: &str) -> Vec<LspSymbolInformation> {
        LspFoundation::get_symbols_for_file(self, file)
    }

    fn add_diagnostic(&mut self, diagnostic: LspDiagnostic) {
        LspFoundation::add_diagnostic(self, diagnostic);
    }

    fn get_diagnostics_for_file(&self, file: &str) -> Vec<LspDiagnostic> {
        LspFoundation::get_diagnostics_for_file(self, file)
    }

    fn find_definition(&self, uri: &str, position: &LspPosition) -> Option<LspLocation> {
        LspFoundation::find_definition(self, uri, position)
    }

    fn find_references(&self, symbol_name: &str) -> Vec<LspLocation> {
        LspFoundation::find_references(self, symbol_name)
    }

    fn rename_symbol(
        &self,
        uri: &str,
        position: &LspPosition,
        new_name: &str,
    ) -> Option<LspWorkspaceEdit> {
        LspFoundation::rename_symbol(self, uri, position, new_name)
    }
}

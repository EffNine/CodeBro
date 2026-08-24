#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSymbolInformation {
    pub name: String,
    pub kind: SymbolKind,
    pub location: LspLocation,
    pub container_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SymbolKind {
    File = 1,
    Module = 2,
    Namespace = 3,
    Package = 4,
    Class = 5,
    Method = 6,
    Property = 7,
    Field = 8,
    Constructor = 9,
    Enum = 10,
    Interface = 11,
    Function = 12,
    Variable = 13,
    Constant = 14,
    String = 15,
    Number = 16,
    Boolean = 17,
    Array = 18,
    Object = 19,
    Key = 20,
    Null = 21,
    EnumMember = 22,
    Struct = 23,
    Event = 24,
    Operator = 25,
    TypeParameter = 26,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspTextEdit {
    pub range: LspRange,
    pub new_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspWorkspaceEdit {
    pub changes: Option<HashMap<String, Vec<LspTextEdit>>>,
    pub document_changes: Option<Vec<LspDocumentEdit>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDocumentEdit {
    pub text_document: LspTextDocumentIdentifier,
    pub edits: Vec<LspTextEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspTextDocumentIdentifier {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspTextDocumentItem {
    pub uri: String,
    pub language_id: String,
    pub version: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCompletionItem {
    pub label: String,
    pub kind: Option<SymbolKind>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub text_edit: Option<LspTextEdit>,
    pub insert_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHover {
    pub contents: String,
    pub range: Option<LspRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSignatureInformation {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Option<Vec<LspParameterInformation>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspParameterInformation {
    pub label: String,
    pub documentation: Option<String>,
}

pub struct LspFoundation {
    pub open_documents: HashMap<String, LspTextDocumentItem>,
    pub symbols: Vec<LspSymbolInformation>,
    pub diagnostics: Vec<LspDiagnostic>,
}

impl LspFoundation {
    pub fn new() -> Self {
        LspFoundation {
            open_documents: HashMap::new(),
            symbols: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn open_document(&mut self, document: LspTextDocumentItem) {
        self.open_documents.insert(document.uri.clone(), document);
    }

    pub fn close_document(&mut self, uri: &str) {
        self.open_documents.remove(uri);
    }

    pub fn get_document(&self, uri: &str) -> Option<&LspTextDocumentItem> {
        self.open_documents.get(uri)
    }

    pub fn update_document(&mut self, uri: &str, text: String, version: i32) {
        if let Some(doc) = self.open_documents.get_mut(uri) {
            doc.text = text;
            doc.version = version;
        }
    }

    pub fn get_text(&self, uri: &str) -> Option<String> {
        self.open_documents.get(uri).map(|d| d.text.clone())
    }

    pub fn add_symbol(&mut self, symbol: LspSymbolInformation) {
        self.symbols.push(symbol);
    }

    pub fn add_diagnostic(&mut self, diagnostic: LspDiagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn get_symbols_for_file(&self, file: &str) -> Vec<LspSymbolInformation> {
        self.symbols
            .iter()
            .filter(|s| s.location.uri.contains(file))
            .cloned()
            .collect()
    }

    pub fn get_diagnostics_for_file(&self, _file: &str) -> Vec<LspDiagnostic> {
        Vec::new()
    }

    pub fn find_definition(&self, uri: &str, position: &LspPosition) -> Option<LspLocation> {
        for symbol in &self.symbols {
            if symbol.location.uri == uri {
                if symbol.location.range.start.line == position.line {
                    if position.character >= symbol.location.range.start.character
                        && position.character <= symbol.location.range.end.character
                    {
                        return Some(symbol.location.clone());
                    }
                }
            }
        }
        None
    }

    pub fn find_references(&self, symbol_name: &str) -> Vec<LspLocation> {
        self.symbols
            .iter()
            .filter(|s| s.name == symbol_name)
            .map(|s| s.location.clone())
            .collect()
    }

    pub fn rename_symbol(
        &self,
        uri: &str,
        position: &LspPosition,
        new_name: &str,
    ) -> Option<LspWorkspaceEdit> {
        let _old_symbol = self.find_definition(uri, position)?;
        let old_name = self.get_symbol_name_at(uri, position)?;

        let mut changes: HashMap<String, Vec<LspTextEdit>> = HashMap::new();

        for symbol in &self.symbols {
            if symbol.name == old_name {
                let edit = LspTextEdit {
                    range: symbol.location.range.clone(),
                    new_text: new_name.to_string(),
                };

                changes
                    .entry(symbol.location.uri.clone())
                    .or_default()
                    .push(edit);
            }
        }

        Some(LspWorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
        })
    }

    fn get_symbol_name_at(&self, _uri: &str, _position: &LspPosition) -> Option<String> {
        None
    }
}

pub fn create_lsp_foundation() -> LspFoundation {
    LspFoundation::new()
}

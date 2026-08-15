#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};

use std::path::Path;
use tree_sitter::{Language, Node, Parser, Point};

use crate::intelligence::parser::languages;

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Method,
    Variable,
    Constant,
    TypeAlias,
    Module,
    Import,
    Export,
    Field,
    Parameter,
    Macro,
    Impl,
    Constructor,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "function"),
            SymbolKind::Class => write!(f, "class"),
            SymbolKind::Struct => write!(f, "struct"),
            SymbolKind::Enum => write!(f, "enum"),
            SymbolKind::Trait => write!(f, "trait"),
            SymbolKind::Interface => write!(f, "interface"),
            SymbolKind::Method => write!(f, "method"),
            SymbolKind::Variable => write!(f, "variable"),
            SymbolKind::Constant => write!(f, "constant"),
            SymbolKind::TypeAlias => write!(f, "type_alias"),
            SymbolKind::Module => write!(f, "module"),
            SymbolKind::Import => write!(f, "import"),
            SymbolKind::Export => write!(f, "export"),
            SymbolKind::Field => write!(f, "field"),
            SymbolKind::Parameter => write!(f, "parameter"),
            SymbolKind::Macro => write!(f, "macro"),
            SymbolKind::Impl => write!(f, "impl"),
            SymbolKind::Constructor => write!(f, "constructor"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub language: String,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub column_start: u32,
    pub column_end: u32,
    pub parent: Option<String>,
    pub visibility: Option<String>,
    pub signature: Option<String>,
    pub doc_comment: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub errors: Vec<String>,
}

pub struct CodeParser {
    parser: Parser,
    language: Language,
    language_name: String,
}

impl CodeParser {
    pub fn new(language: &str) -> Result<Self> {
        let lang = languages::get_language(language)
            .with_context(|| format!("Unsupported language: {}", language))?;

        let mut parser = Parser::new();
        parser
            .set_language(lang)
            .with_context(|| format!("Failed to set language for {}", language))?;

        Ok(CodeParser {
            parser,
            language: lang,
            language_name: language.to_string(),
        })
    }

    pub fn parse_file(&mut self, file_path: &Path, source: &str) -> Result<ParseResult> {
        let tree = self
            .parser
            .parse(source, None)
            .context("Failed to parse source code")?;

        let mut result = ParseResult {
            symbols: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            errors: Vec::new(),
        };

        let root = tree.root_node();
        self.extract_symbols(root, source, file_path, &mut result, None)?;

        Ok(result)
    }

    pub fn parse_source(&mut self, source: &str, file_path: &str) -> Result<ParseResult> {
        let tree = self
            .parser
            .parse(source, None)
            .context("Failed to parse source code")?;

        let mut result = ParseResult {
            symbols: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            errors: Vec::new(),
        };

        let root = tree.root_node();
        self.extract_symbols(root, source, &Path::new(file_path), &mut result, None)?;

        Ok(result)
    }

    fn extract_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_path: &Path,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) -> Result<()> {
        let _node_kind = node.kind();
        let file_name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        match self.language_name.as_str() {
            "rust" => self.extract_rust_symbols(node, source, &file_name, result, parent)?,
            "python" => self.extract_python_symbols(node, source, &file_name, result, parent)?,
            "javascript" | "jsx" => {
                self.extract_js_symbols(node, source, &file_name, result, parent)?
            }
            "typescript" | "tsx" => {
                self.extract_ts_symbols(node, source, &file_name, result, parent)?
            }
            "go" => self.extract_go_symbols(node, source, &file_name, result, parent)?,
            _ => {}
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_symbols(child, source, file_path, result, parent)?;
            }
        }

        Ok(())
    }

    fn node_text(&self, node: Node, source: &str) -> String {
        let start = node.start_byte();
        let end = node.end_byte();
        source[start..end].to_string()
    }

    fn node_name(&self, node: Node, source: &str) -> Option<String> {
        let text = self.node_text(node, source).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Resolve a declaration's name. Most grammars name declarations via
    /// `identifier`; Rust (and TypeScript) type declarations use
    /// `type_identifier` instead, so fall back to it when the primary kind
    /// is absent. `get_node_by_kind` searches direct children only, which
    /// keeps field/variable names out of type-name lookups.
    fn name_of(&self, node: Node, source: &str, kind: &str) -> String {
        self.get_node_by_kind(node, kind)
            .and_then(|n| self.node_name(n, source))
            .unwrap_or_else(|| {
                self.get_node_by_kind(node, "type_identifier")
                    .and_then(|n| self.node_name(n, source))
                    .unwrap_or_else(|| "unknown".to_string())
            })
    }

    fn get_node_by_kind<'a>(&self, node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == kind {
                    return Some(child);
                }
            }
        }
        None
    }

    fn find_children_by_kind<'a>(&self, node: Node<'a>, kind: &str) -> Vec<Node<'a>> {
        let mut results = Vec::new();
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == kind {
                    results.push(child);
                }
                results.extend(self.find_children_by_kind(child, kind));
            }
        }
        results
    }

    fn visibility_from_modifiers(&self, node: Node, source: &str) -> Option<String> {
        let text = self.node_text(node, source);
        if text.contains("pub") {
            Some("public".to_string())
        } else if text.contains("priv") || text.contains("pub(crate)") {
            Some("crate".to_string())
        } else {
            None
        }
    }

    fn line_to_u32(&self, point: Point) -> u32 {
        point.row as u32 + 1
    }

    fn extract_rust_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) -> Result<()> {
        // Skip unnamed nodes (keywords, punctuation, operators). Without this,
        // the keyword children of item nodes (e.g. `struct`, `fn`, `impl`)
        // match the item arms below and produce duplicate "unknown" symbols.
        if !node.is_named() {
            return Ok(());
        }
        let kind = node.kind();

        match kind {
            "fn" | "function_item" => {
                let name = self.name_of(node, source, "identifier");

                let sig = self.extract_signature(node, source);

                let visibility = if node.child_count() > 0 {
                    self.visibility_from_modifiers(node, source)
                } else {
                    None
                };

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Function,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility,
                    signature: sig,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "struct" | "struct_item" => {
                let name = self.name_of(node, source, "identifier");

                let visibility = self.visibility_from_modifiers(node, source);

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Struct,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "enum" | "enum_item" => {
                let name = self.name_of(node, source, "identifier");

                let visibility = self.visibility_from_modifiers(node, source);

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Enum,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "trait" | "trait_item" => {
                let name = self.name_of(node, source, "identifier");

                let visibility = self.visibility_from_modifiers(node, source);

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Trait,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "impl" | "impl_item" => {
                let name = self.name_of(node, source, "type_identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Impl,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: None,
                });
            }
            "type" | "type_alias" | "type_item" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::TypeAlias,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: None,
                });
            }
            "macro" | "macro_item" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Macro,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: None,
                });
            }
            "mod" | "mod_item" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Module,
                    language: "rust".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: None,
                });
            }
            "use" | "use_item" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
            }
            _ => {}
        }

        Ok(())
    }

    fn extract_python_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) -> Result<()> {
        let kind = node.kind();

        match kind {
            "function_definition" => {
                let name = self.name_of(node, source, "identifier");

                let sig = self.extract_signature(node, source);

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Function,
                    language: "python".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: sig,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "class_definition" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Class,
                    language: "python".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "import_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
            }
            "import_from_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
            }
            _ => {}
        }

        Ok(())
    }

    fn extract_js_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) -> Result<()> {
        let kind = node.kind();

        match kind {
            "function_declaration" | "function_expression" | "arrow_function" => {
                let name = self.name_of(node, source, "identifier");

                let sig = self.extract_signature(node, source);

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Function,
                    language: "javascript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: sig,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "class_declaration" | "class_expression" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Class,
                    language: "javascript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "method_definition" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Method,
                    language: "javascript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: self.extract_signature(node, source),
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "import_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
            }
            "export_statement" => {
                let text = self.node_text(node, source);
                result.exports.push(text);
            }
            _ => {}
        }

        Ok(())
    }

    fn extract_ts_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) -> Result<()> {
        let kind = node.kind();

        match kind {
            "function_declaration" | "function_expression" | "arrow_function" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Function,
                    language: "typescript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: self.extract_signature(node, source),
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "class_declaration" | "class_expression" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Class,
                    language: "typescript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "method_definition" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Method,
                    language: "typescript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: self.extract_signature(node, source),
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "interface_declaration" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Interface,
                    language: "typescript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "import_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
            }
            "export_statement" => {
                let text = self.node_text(node, source);
                result.exports.push(text);
            }
            "type_alias_declaration" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::TypeAlias,
                    language: "typescript".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: None,
                    doc_comment: None,
                });
            }
            _ => {}
        }

        Ok(())
    }

    fn extract_go_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) -> Result<()> {
        let kind = node.kind();

        match kind {
            "function_declaration" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Function,
                    language: "go".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: self.extract_signature(node, source),
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "type_declaration" => {
                let type_spec = self.get_node_by_kind(node, "type_spec");
                if let Some(ts) = type_spec {
                    let name_node = self.get_node_by_kind(ts, "identifier");
                    let name = name_node
                        .and_then(|n| self.node_name(n, source))
                        .unwrap_or_else(|| "unknown".to_string());

                    let type_node = self.get_node_by_kind(ts, "struct_type");
                    let kind = if type_node.is_some() {
                        SymbolKind::Struct
                    } else {
                        SymbolKind::TypeAlias
                    };

                    result.symbols.push(ParsedSymbol {
                        name,
                        kind,
                        language: "go".to_string(),
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        line_end: self.line_to_u32(node.end_position()),
                        column_start: node.start_position().column as u32,
                        column_end: node.end_position().column as u32,
                        parent: parent.map(|s| s.to_string()),
                        visibility: None,
                        signature: None,
                        doc_comment: self.extract_doc_comment(node, source),
                    });
                }
            }
            "method_declaration" => {
                let name = self.name_of(node, source, "identifier");

                result.symbols.push(ParsedSymbol {
                    name,
                    kind: SymbolKind::Method,
                    language: "go".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: parent.map(|s| s.to_string()),
                    visibility: None,
                    signature: self.extract_signature(node, source),
                    doc_comment: self.extract_doc_comment(node, source),
                });
            }
            "interface_type" => {
                let name_node = self.get_node_by_kind(node, "identifier");
                if let Some(name_node) = name_node {
                    let name = self
                        .node_name(name_node, source)
                        .unwrap_or_else(|| "unknown".to_string());

                    result.symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Interface,
                        language: "go".to_string(),
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        line_end: self.line_to_u32(node.end_position()),
                        column_start: node.start_position().column as u32,
                        column_end: node.end_position().column as u32,
                        parent: parent.map(|s| s.to_string()),
                        visibility: None,
                        signature: None,
                        doc_comment: None,
                    });
                }
            }
            "struct_type" => {
                let name_node = self.get_node_by_kind(node, "identifier");
                if let Some(name_node) = name_node {
                    let name = self
                        .node_name(name_node, source)
                        .unwrap_or_else(|| "unknown".to_string());

                    result.symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Struct,
                        language: "go".to_string(),
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        line_end: self.line_to_u32(node.end_position()),
                        column_start: node.start_position().column as u32,
                        column_end: node.end_position().column as u32,
                        parent: parent.map(|s| s.to_string()),
                        visibility: None,
                        signature: None,
                        doc_comment: None,
                    });
                }
            }
            "import_declaration" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
            }
            _ => {}
        }

        Ok(())
    }

    fn extract_signature(&self, node: Node, source: &str) -> Option<String> {
        let mut sig_parts = Vec::new();
        self.collect_signature_text(node, source, &mut sig_parts);
        if sig_parts.is_empty() {
            None
        } else {
            Some(sig_parts.join(" "))
        }
    }

    fn collect_signature_text(&self, node: Node, source: &str, parts: &mut Vec<String>) {
        let text = self.node_text(node, source).trim().to_string();
        if !text.is_empty() && text.len() < 200 {
            parts.push(text);
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_signature_text(child, source, parts);
            }
        }
    }

    fn extract_doc_comment(&self, node: Node, source: &str) -> Option<String> {
        let prev_sibling = node.prev_sibling();
        if let Some(prev) = prev_sibling {
            let prev_kind = prev.kind();
            if prev_kind == "comment" || prev_kind == "line_comment" || prev_kind == "block_comment"
            {
                let text = self.node_text(prev, source);
                let cleaned = text
                    .lines()
                    .map(|l| {
                        let trimmed = l.trim();
                        if trimmed.starts_with("///") || trimmed.starts_with("//!") {
                            trimmed[3..].trim().to_string()
                        } else if trimmed.starts_with("/*") && trimmed.ends_with("*/") {
                            trimmed[2..trimmed.len() - 2].trim().to_string()
                        } else if trimmed.starts_with("//") {
                            trimmed[2..].trim().to_string()
                        } else {
                            trimmed.to_string()
                        }
                    })
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
        }
        None
    }

    pub fn language_name(&self) -> &str {
        &self.language_name
    }
}

pub fn create_parser(language: &str) -> Result<CodeParser> {
    CodeParser::new(language)
}

pub fn parse_file(language: &str, file_path: &Path, source: &str) -> Result<ParseResult> {
    let mut parser = CodeParser::new(language)?;
    parser.parse_file(file_path, source)
}

pub fn parse_source(language: &str, source: &str, file_path: &str) -> Result<ParseResult> {
    let mut parser = CodeParser::new(language)?;
    parser.parse_source(source, file_path)
}

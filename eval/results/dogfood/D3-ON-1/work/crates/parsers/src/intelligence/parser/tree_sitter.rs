#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};

use std::path::Path;
use tree_sitter::{Language, Node, Parser, Point};

use crate::intelligence::parser::languages;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    #[serde(default)]
    pub is_test: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub errors: Vec<String>,
    /// AST-derived call expressions: caller location + resolved callee name.
    pub calls: Vec<ParseCall>,
    /// AST-derived import targets with structured path info.
    pub import_targets: Vec<ParseImport>,
}

/// A call expression found in the AST.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParseCall {
    /// The module-relative file path of the caller.
    pub caller_file: String,
    /// The symbol name of the function containing this call.
    pub caller_symbol: Option<String>,
    /// Line number (1-based) of the call expression.
    pub line_start: u32,
    /// The callee identifier text extracted from the AST.
    pub callee_name: String,
    /// Whether the callee name could be resolved to a known symbol pattern.
    pub is_qualified: bool,
    /// Receiver type hint: the qualifier of a path call (`User` in
    /// `User::new()`), `"self"`/`"Self"` for self-calls, else `None`.
    pub receiver_type: Option<String>,
    /// True when this call was recovered from macro token text rather
    /// than a real AST call node (tree-sitter 0.20 models macro bodies as
    /// opaque tokens). Such calls are syntactic evidence only and are
    /// tagged `provenance=heuristic` downstream.
    #[serde(default)]
    pub from_macro_text: bool,
}

/// A structured import target extracted from an import/use statement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParseImport {
    /// The module-relative file path.
    pub file: String,
    /// Line number (1-based).
    pub line_start: u32,
    /// The import path as a dot-separated string (e.g. "std::collections::HashMap").
    pub path: String,
    /// For Rust `use` statements: the local alias if present, else None.
    pub alias: Option<String>,
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
            calls: Vec::new(),
            import_targets: Vec::new(),
        };

        let root = tree.root_node();
        let mut bindings = std::collections::HashMap::new();
        self.extract_symbols(root, source, file_path, &mut result, None, &mut bindings)?;

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
            calls: Vec::new(),
            import_targets: Vec::new(),
        };

        let root = tree.root_node();
        let mut bindings = std::collections::HashMap::new();
        self.extract_symbols(
            root,
            source,
            &Path::new(file_path),
            &mut result,
            None,
            &mut bindings,
        )?;

        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn extract_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_path: &Path,
        result: &mut ParseResult,
        parent: Option<&str>,
        bindings: &mut std::collections::HashMap<String, String>,
    ) -> Result<()> {
        let _node_kind = node.kind();
        let file_name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Let-binding scopes reset at callable boundaries so a binding from
        // one function can never resolve a receiver in a sibling function.
        if matches!(
            node.kind(),
            "fn" | "function_item"
                | "function_definition"
                | "function_declaration"
                | "method_declaration"
                | "method_definition"
        ) {
            bindings.clear();
        }

        match self.language_name.as_str() {
            "rust" => {
                self.extract_rust_symbols(node, source, &file_name, result, parent, bindings)?
            }
            "python" => {
                self.extract_python_symbols(node, source, &file_name, result, parent, bindings)?
            }
            "javascript" | "jsx" => {
                self.extract_js_symbols(node, source, &file_name, result, parent, bindings)?
            }
            "typescript" | "tsx" => {
                self.extract_ts_symbols(node, source, &file_name, result, parent, bindings)?
            }
            "go" => self.extract_go_symbols(node, source, &file_name, result, parent, bindings)?,
            _ => {}
        }

        // Same-body let-binding capture (see receiver resolution inside
        // extract_call). Node kinds are language-unique, so this central
        // dispatch stays unambiguous.
        match node.kind() {
            "let_declaration" => self.capture_rust_let(node, source, bindings),
            "assignment" => self.capture_python_binding(node, source, bindings),
            "variable_declarator" => self.capture_js_binding(node, source, bindings),
            _ => {}
        }

        let enclosing = self.enclosing_callable(node, source, parent);
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_symbols(
                    child,
                    source,
                    file_path,
                    result,
                    enclosing.as_deref(),
                    bindings,
                )?;
            }
        }
        Ok(())
    }

    /// The nearest enclosing callable name for a node, used to attribute
    /// extracted calls to their containing function. Function-like nodes
    /// become the enclosing name for everything beneath them; every other
    /// node inherits its parent's context.
    fn enclosing_callable(&self, node: Node, source: &str, parent: Option<&str>) -> Option<String> {
        match node.kind() {
            "fn"
            | "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_declaration" => {
                let name = self.name_of(node, source, "identifier");
                (name != "unknown").then_some(name)
            }
            // JS/TS method names are `property_identifier`, not `identifier`.
            "method_definition" => {
                let name = self
                    .get_node_by_kind(node, "identifier")
                    .or_else(|| self.get_node_by_kind(node, "property_identifier"))
                    .and_then(|n| self.node_name(n, source))
                    .unwrap_or_else(|| "unknown".to_string());
                (name != "unknown").then_some(name)
            }
            _ => parent.map(|p| p.to_string()),
        }
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
        let vis = self.get_node_by_kind(node, "visibility_modifier")?;
        let text = self.node_text(vis, source);
        if text.starts_with("pub(") {
            Some(text)
        } else if text.starts_with("pub") {
            Some("pub".to_string())
        } else {
            None
        }
    }

    /// Rust test marker: a `#[test]` / `#[tokio::test]`-style attribute on
    /// the item directly above the function. Attribute items are siblings,
    /// so walk backwards until a non-attribute sibling appears.
    fn rust_fn_is_test(&self, node: Node, source: &str) -> bool {
        let mut cur = node.prev_sibling();
        while let Some(n) = cur {
            match n.kind() {
                "attribute_item" => {
                    let text = self.node_text(n, source);
                    if text.contains("#[test]")
                        || text.trim_end().trim_end_matches(']').rsplit("::").next() == Some("test")
                    {
                        return true;
                    }
                }
                "line_comment" | "block_comment" => {}
                _ => break,
            }
            cur = n.prev_sibling();
        }
        false
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
        bindings: &mut std::collections::HashMap<String, String>,
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
                    is_test: self.rust_fn_is_test(node, source),
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
                    is_test: false,
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
                    is_test: false,
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
                    is_test: false,
                });
            }
            "impl" | "impl_item" => {
                // Prefer the LAST direct type identifier: for
                // `impl Display for Foo` the self type is Foo, not the
                // trait; for plain `impl Foo` it is Foo either way.
                let name = node
                    .children(&mut node.walk())
                    .filter(|c| c.kind() == "type_identifier")
                    .last()
                    .and_then(|n| self.node_name(n, source))
                    .unwrap_or_else(|| "unknown".to_string());

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
                    is_test: false,
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
                    is_test: false,
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
                    is_test: false,
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
                    is_test: false,
                });
            }
            "use" | "use_item" | "use_declaration" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
                // Also extract a structured import target for relationship
                // building. Parse the path from the use statement.
                if let Some(path) = self.parse_use_path(node, source) {
                    result.import_targets.push(ParseImport {
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        path,
                        alias: None,
                    });
                }
            }
            "call_expression" => {
                self.extract_call(node, source, file_name, result, parent, bindings);
            }
            "token_tree" => {
                self.extract_macro_text_calls(node, source, file_name, result, parent);
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
        bindings: &mut std::collections::HashMap<String, String>,
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
                    is_test: false,
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
                    is_test: false,
                });
            }
            "import_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
                self.parse_python_import_statement(node, source, file_name, result);
            }
            "import_from_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
                self.parse_python_import_from(node, source, file_name, result);
            }
            "call" => {
                self.extract_call(node, source, file_name, result, parent, bindings);
            }
            _ => {}
        }

        Ok(())
    }

    /// `import a.b.c as x, y` — one structured target per dotted name.
    fn parse_python_import_statement(
        &self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
    ) {
        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else { continue };
            match child.kind() {
                "dotted_name" => {
                    let path = self.node_text(child, source);
                    result.import_targets.push(ParseImport {
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        path,
                        alias: None,
                    });
                }
                "aliased_import" => {
                    let Some(name) = child.child_by_field_name("name") else {
                        continue;
                    };
                    let alias = child
                        .child_by_field_name("alias")
                        .and_then(|a| self.node_name(a, source));
                    result.import_targets.push(ParseImport {
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        path: self.node_text(name, source),
                        alias,
                    });
                }
                _ => {}
            }
        }
    }

    /// `from X import y` — resolve the MODULE (X) as the import target; the
    /// imported names are symbols inside it and do not map to module files.
    /// Relative imports keep their dots (`".helpers"`); last-segment module
    /// matching still resolves them.
    fn parse_python_import_from(
        &self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
    ) {
        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else { continue };
            if matches!(child.kind(), "dotted_name" | "relative_import") {
                // Only the FIRST module reference counts; later dotted names
                // are the imported symbol list.
                result.import_targets.push(ParseImport {
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    path: self.node_text(child, source),
                    alias: None,
                });
                return;
            }
        }
    }

    fn extract_js_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
        bindings: &mut std::collections::HashMap<String, String>,
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
                    is_test: false,
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
                    is_test: false,
                });
            }
            "method_definition" => {
                let name = self
                    .get_node_by_kind(node, "identifier")
                    .or_else(|| self.get_node_by_kind(node, "property_identifier"))
                    .and_then(|n| self.node_name(n, source))
                    .unwrap_or_else(|| "unknown".to_string());

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
                    is_test: false,
                });
            }
            "import_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
                self.push_js_module_import(node, source, file_name, result);
            }
            "export_statement" => {
                let text = self.node_text(node, source);
                result.exports.push(text);
                self.push_js_module_import(node, source, file_name, result);
            }
            "call_expression" => {
                self.extract_call(node, source, file_name, result, parent, bindings);
            }
            "new_expression" => {
                self.extract_js_new_call(node, source, file_name, result, parent);
            }
            _ => {}
        }

        Ok(())
    }

    /// Extract an `import`/`export ... from "<module>"` source specifier.
    fn push_js_module_import(
        &self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
    ) {
        let Some(src) = node.child_by_field_name("source") else {
            return;
        };
        let raw = self.node_text(src, source);
        let path = raw
            .trim_matches(|c| c == '"' || c == '\'')
            .trim()
            .to_string();
        if path.is_empty() {
            return;
        }
        // Bare external specifiers ("react", "lodash-es") cannot resolve to
        // a workspace module; only relative/scoped/path-ish targets are
        // worth structured tracking.
        if !(path.starts_with('.') || path.contains('/') || path.starts_with('@')) {
            return;
        }
        let cleaned = path.strip_prefix("./").unwrap_or(&path).to_string();
        result.import_targets.push(ParseImport {
            file: file_name.to_string(),
            line_start: self.line_to_u32(node.start_position()),
            path: cleaned,
            alias: None,
        });
    }

    /// `new Foo()` / `new ns.Bar()` — constructors are impact-relevant
    /// callees; capture them like calls on the constructor identifier.
    fn extract_js_new_call(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let Some(ctor) = node.child_by_field_name("constructor") else {
            return;
        };
        let callee_name = self.extract_callee_name(ctor, source);
        if callee_name.is_empty() {
            return;
        }
        result.calls.push(ParseCall {
            caller_file: file_name.to_string(),
            caller_symbol: parent.map(|s| s.to_string()),
            line_start: self.line_to_u32(node.start_position()),
            callee_name,
            is_qualified: ctor.kind() == "member_expression",
            receiver_type: self.extract_receiver_type(ctor, source),
            from_macro_text: false,
        });
    }

    fn extract_ts_symbols(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
        bindings: &mut std::collections::HashMap<String, String>,
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
                    is_test: false,
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
                    is_test: false,
                });
            }
            "method_definition" => {
                let name = self
                    .get_node_by_kind(node, "identifier")
                    .or_else(|| self.get_node_by_kind(node, "property_identifier"))
                    .and_then(|n| self.node_name(n, source))
                    .unwrap_or_else(|| "unknown".to_string());

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
                    is_test: false,
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
                    is_test: false,
                });
            }
            "import_statement" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
                self.push_js_module_import(node, source, file_name, result);
            }
            "export_statement" => {
                let text = self.node_text(node, source);
                result.exports.push(text);
                self.push_js_module_import(node, source, file_name, result);
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
                    is_test: false,
                });
            }
            "call_expression" => {
                self.extract_call(node, source, file_name, result, parent, bindings);
            }
            "new_expression" => {
                self.extract_js_new_call(node, source, file_name, result, parent);
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
        bindings: &mut std::collections::HashMap<String, String>,
    ) -> Result<()> {
        let kind = node.kind();

        match kind {
            "function_declaration" => {
                let name = self.name_of(node, source, "identifier");
                let is_test = name.starts_with("Test");

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
                    is_test,
                });
            }
            "method_declaration" => {
                // Go methods: `func (b *Breaker) Stats() ...`. The method
                // name is a `field_identifier` (not `identifier`), and the
                // receiver type is the parent context.
                let name = self
                    .get_node_by_kind(node, "field_identifier")
                    .and_then(|n| self.node_name(n, source))
                    .unwrap_or_else(|| "unknown".to_string());

                // Receiver type from the first parameter list: (b *Breaker).
                let receiver = self
                    .get_node_by_kind(node, "parameter_list")
                    .and_then(|pl| {
                        (0..pl.child_count()).find_map(|i| {
                            let c = pl.child(i)?;
                            if c.kind() == "parameter_declaration" {
                                Some(c)
                            } else {
                                None
                            }
                        })
                    })
                    .and_then(|pd| {
                        (0..pd.child_count()).find_map(|i| {
                            let c = pd.child(i)?;
                            if c.kind() == "pointer_type" || c.kind() == "type_identifier" {
                                Some(self.node_text(c, source))
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or_default();

                result.symbols.push(ParsedSymbol {
                    name: name.clone(),
                    kind: SymbolKind::Method,
                    language: "go".to_string(),
                    file: file_name.to_string(),
                    line_start: self.line_to_u32(node.start_position()),
                    line_end: self.line_to_u32(node.end_position()),
                    column_start: node.start_position().column as u32,
                    column_end: node.end_position().column as u32,
                    parent: Some(if receiver.is_empty() {
                        name.clone()
                    } else {
                        receiver
                    }),
                    visibility: None,
                    signature: self.extract_signature(node, source),
                    doc_comment: self.extract_doc_comment(node, source),
                    is_test: false,
                });
            }
            "type_declaration" => {
                // `type X struct{...}` / `type X interface{...}` / `type X int32`
                // are `type_spec`; `type X = Y` is a dedicated `type_alias`
                // node in tree-sitter-go. Handle both.
                let ts = self
                    .get_node_by_kind(node, "type_spec")
                    .or_else(|| self.get_node_by_kind(node, "type_alias"));
                if let Some(ts) = ts {
                    // Go names type declarations via `type_identifier`, not
                    // `identifier`; `name_of` falls back to it so structs,
                    // interfaces, aliases and defined types keep real names.
                    let name = self.name_of(ts, source, "identifier");

                    let type_node = self.get_node_by_kind(ts, "struct_type");
                    let iface_node = self.get_node_by_kind(ts, "interface_type");
                    let kind = if type_node.is_some() {
                        SymbolKind::Struct
                    } else if iface_node.is_some() {
                        SymbolKind::Interface
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
                        is_test: false,
                    });
                }
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
                        is_test: false,
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
                        is_test: false,
                    });
                }
            }
            "import_declaration" => {
                let text = self.node_text(node, source);
                result.imports.push(text);
                // Parse structured import targets.
                if let Some((path, alias)) = self.parse_go_import(node, source) {
                    result.import_targets.push(ParseImport {
                        file: file_name.to_string(),
                        line_start: self.line_to_u32(node.start_position()),
                        path,
                        alias,
                    });
                }
            }
            "call_expression" => {
                self.extract_call(node, source, file_name, result, parent, bindings);
            }
            _ => {}
        }

        Ok(())
    }

    /// Extract a clean, syntactically recognizable declaration signature.
    ///
    /// The signature is the source text from the declaration start up to the
    /// start of its body (block / statement_block / compound_statement), or up
    /// to the `=>` for arrow functions. Cutting at the AST body boundary keeps
    /// the signature readable instead of replaying every descendant node's text
    /// (which produced garbled output like `func (m *mockProvider) ( m
    /// *mockProvider)`).
    fn extract_signature(&self, node: Node, source: &str) -> Option<String> {
        let mut end = node.end_byte();
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "block" | "statement_block" | "compound_statement" | "body" => {
                        end = child.start_byte();
                        break;
                    }
                    "=>" => {
                        end = child.end_byte();
                        break;
                    }
                    _ => {}
                }
            }
        }

        let raw = &source[node.start_byte()..end];
        // Collapse whitespace runs (incl. newlines in multi-line parameter
        // lists) to single spaces for a compact, LLM-friendly signature.
        let sig: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let sig = sig.trim().to_string();
        if sig.is_empty() {
            return None;
        }

        // Safety cap for declarations without a body node (e.g. large arrow
        // function expression bodies): keep a bounded, deterministic prefix.
        const MAX_SIGNATURE_CHARS: usize = 512;
        if sig.len() > MAX_SIGNATURE_CHARS {
            let cut = sig.floor_char_boundary(MAX_SIGNATURE_CHARS);
            return Some(format!("{} …", &sig[..cut]));
        }
        Some(sig)
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

    /// Extract a call expression from the AST. Populates `result.calls`
    /// with the callee identifier and location. Only handles simple
    /// identifier calls and qualified selector calls where the final
    /// segment is an identifier.
    fn extract_call(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
        bindings: &std::collections::HashMap<String, String>,
    ) {
        // The callee is the first child of a call_expression.
        let Some(callee) = node.child(0) else {
            return;
        };
        let callee_name = self.extract_callee_name(callee, source);
        if callee_name.is_empty() {
            return;
        }
        let is_qualified = matches!(
            callee.kind(),
            "field_expression"
                | "selector_expression"
                | "qualified_identifier"
                | "scoped_identifier"
                | "attribute"
                | "member_expression"
        );
        let mut receiver_type = self.extract_receiver_type(callee, source);
        // Same-body let-binding inference: `let p = User::new(); p.save()`
        // resolves `p` to `User` when the binding was captured from a
        // constructor-shaped initializer. Plain variables stay untyped
        // otherwise — never invent a type from a bare name.
        if receiver_type.is_none() && is_qualified {
            if let Some(object) = self.receiver_object_text(callee, source) {
                if let Some(bound) = bindings.get(&object) {
                    receiver_type = Some(bound.clone());
                }
            }
        }
        result.calls.push(ParseCall {
            caller_file: file_name.to_string(),
            caller_symbol: parent.map(|s| s.to_string()),
            line_start: self.line_to_u32(node.start_position()),
            callee_name,
            is_qualified,
            receiver_type,
            from_macro_text: false,
        });
    }

    /// `let p = Type::new(…)` binds `p → Type` from the scoped-path
    /// qualifier. Any other initializer shape carries no trustworthy type.
    fn capture_rust_let(
        &self,
        node: Node,
        source: &str,
        bindings: &mut std::collections::HashMap<String, String>,
    ) {
        // Grammar 0.20 gives let_declaration no field names — extract
        // positionally: the binding pattern is the first identifier child,
        // the initializer the (single) call_expression child.
        let mut pattern = None;
        let mut init = None;
        for i in 0..node.child_count() {
            let Some(c) = node.child(i) else { continue };
            match c.kind() {
                "identifier" if pattern.is_none() => pattern = Some(c),
                "call_expression" => init = Some(c),
                _ => {}
            }
        }
        let (Some(name), Some(value)) = (pattern, init) else {
            return;
        };
        if value.kind() != "call_expression" {
            return;
        }
        let Some(callee) = value.child(0) else {
            return;
        };
        if callee.kind() != "scoped_identifier" {
            return;
        }
        let Some(path) = callee.child_by_field_name("path") else {
            return;
        };
        bindings.insert(self.node_text(name, source), self.node_text(path, source));
    }

    /// `p = User()` binds `p → User` by constructor naming convention
    /// (capitalized callee only — lowercase calls are plain functions).
    fn capture_python_binding(
        &self,
        node: Node,
        source: &str,
        bindings: &mut std::collections::HashMap<String, String>,
    ) {
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            return;
        };
        if right.kind() != "call" {
            return;
        }
        let Some(func) = right.child_by_field_name("function") else {
            return;
        };
        if func.kind() != "identifier" {
            return;
        }
        let name = self.node_text(func, source);
        if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return;
        }
        bindings.insert(self.node_text(left, source), name);
    }

    /// `const p = new Widget()` binds via the constructor; a TS type
    /// annotation (`let p: User`) binds even without an initializer.
    fn capture_js_binding(
        &self,
        node: Node,
        source: &str,
        bindings: &mut std::collections::HashMap<String, String>,
    ) {
        // Positional extraction (grammar 0.20 may omit field names):
        // `name` = first identifier child; initializer = the child after
        // the "=" token, when present.
        let mut name_node = None;
        let mut value_node = None;
        let mut seen_eq = false;
        for i in 0..node.child_count() {
            let Some(c) = node.child(i) else { continue };
            match c.kind() {
                "identifier" if name_node.is_none() && !seen_eq && !c.has_error() => {
                    name_node = Some(c)
                }
                "=" => seen_eq = true,
                _ if seen_eq && value_node.is_none() && c.is_named() => value_node = Some(c),
                _ => {}
            }
        }
        let Some(name_node) = name_node else { return };
        let key = self.node_text(name_node, source);
        match value_node.as_ref().map(|v| v.kind()) {
            Some("new_expression") => {
                let v = value_node.unwrap();
                let Some(ctor) = v.child_by_field_name("constructor") else {
                    return;
                };
                let ctor_name = self.extract_callee_name(ctor, source);
                if !ctor_name.is_empty() {
                    bindings.insert(key, ctor_name);
                }
            }
            _ => {
                let mut cursor = node.walk();
                let ty = node
                    .children(&mut cursor)
                    .find(|c| c.kind() == "type_identifier")
                    .or_else(|| {
                        // TS: the annotation is wrapped in a
                        // `type_annotation` node.
                        node.children(&mut node.walk())
                            .find(|c| c.kind() == "type_annotation")
                            .and_then(|ta| {
                                ta.child_by_field_name("type").or_else(|| {
                                    ta.children(&mut ta.walk())
                                        .find(|c| c.kind() == "type_identifier")
                                })
                            })
                            .map(|t| t.child_by_field_name("type").unwrap_or(t))
                    });
                if let Some(ty) = ty {
                    let ty = match ty.child_by_field_name("type") {
                        Some(inner) => inner,
                        None => ty,
                    };
                    bindings.insert(key, self.node_text(ty, source));
                }
            }
        }
    }

    const RUST_RESERVED: &[&str] = &[
        "if",
        "while",
        "for",
        "match",
        "return",
        "fn",
        "let",
        "unsafe",
        "loop",
        "macro_rules",
        "else",
        "move",
        "ref",
    ];

    /// Recover syntactic calls from macro token text (`assert_eq!(add(2,
    /// 3), 5)`). Tree-sitter 0.20 gives macro bodies no AST, so this scans
    /// the token text for `name(` and `Path::name(` shapes. Reserved
    /// keywords are skipped; results are flagged `from_macro_text` so
    /// relationship building tags them heuristic instead of verified.
    fn extract_macro_text_calls(
        &mut self,
        node: Node,
        source: &str,
        file_name: &str,
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let text = self.node_text(node, source);
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !(bytes[i] == b'_' || bytes[i].is_ascii_alphabetic()) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let word = &text[start..i];
            // Look ahead past whitespace for an opening paren.
            let mut j = i;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] != b'(' {
                continue;
            }
            // Look behind for a `::` qualifier (skip leading whitespace).
            let mut k = start;
            while k > 0 && (bytes[k - 1] as char).is_whitespace() {
                k -= 1;
            }
            let (receiver, qualified) = if k >= 2 && &text[k - 2..k] == "::" {
                let mut e = k - 2;
                while e > 0 && (bytes[e - 1] as char).is_whitespace() {
                    e -= 1;
                }
                let mut s2 = e;
                while s2 > 0 && (bytes[s2 - 1] == b'_' || bytes[s2 - 1].is_ascii_alphanumeric()) {
                    s2 -= 1;
                }
                if s2 < e {
                    (Some(text[s2..e].to_string()), true)
                } else {
                    (None, true)
                }
            } else {
                (None, false)
            };
            if Self::RUST_RESERVED.contains(&word) || word == "self" {
                continue;
            }
            result.calls.push(ParseCall {
                caller_file: file_name.to_string(),
                caller_symbol: parent.map(|s| s.to_string()),
                line_start: self.line_to_u32(node.start_position()),
                callee_name: word.to_string(),
                is_qualified: qualified,
                receiver_type: receiver,
                from_macro_text: true,
            });
        }
    }

    /// The receiver expression text of a qualified callee (`p` in
    /// `p.save()` across languages), used for let-binding lookups.
    fn receiver_object_text(&self, callee: Node, source: &str) -> Option<String> {
        let field = match callee.kind() {
            "attribute" | "member_expression" => "object",
            "field_expression" => "value",
            _ => return None,
        };
        let object = callee.child_by_field_name(field)?;
        Some(self.node_text(object, source))
    }

    /// Receiver-type hint for a call: the qualifier segment of a path call
    /// (`User` in `User::new()`, including `Self`), `"self"` for
    /// instance calls on self, and the equivalent trustworthy receivers in
    /// other languages (`self`/`cls` for Python attributes, `this` for JS/TS
    /// member expressions). Plain variable receivers yield `None` —
    /// without binding information they carry no trustworthy type.
    fn extract_receiver_type(&self, callee: Node, source: &str) -> Option<String> {
        match callee.kind() {
            "scoped_identifier" | "qualified_identifier" => {
                let path = callee.child_by_field_name("path")?;
                self.last_path_segment_text(path, source)
            }
            "field_expression" => {
                let value = callee.child_by_field_name("value")?;
                let text = self.node_text(value, source);
                (text == "self").then(|| "self".to_string())
            }
            "attribute" => {
                let object = callee.child_by_field_name("object")?;
                let text = self.node_text(object, source);
                (text == "self" || text == "cls").then_some(text)
            }
            "member_expression" => {
                let object = callee.child_by_field_name("object")?;
                let text = self.node_text(object, source);
                (text == "this").then(|| "this".to_string())
            }
            _ => None,
        }
    }

    /// Final identifier of a (possibly nested) path node: follows the
    /// `name` field through nested scoped identifiers.
    fn last_path_segment_text(&self, node: Node, source: &str) -> Option<String> {
        if let Some(name) = node.child_by_field_name("name") {
            return self.last_path_segment_text(name, source);
        }
        match node.kind() {
            "identifier" | "type_identifier" => self.node_name(node, source),
            _ => None,
        }
    }

    /// Walk a callee node tree and return the final identifier text.
    fn extract_callee_name(&self, node: Node, source: &str) -> String {
        match node.kind() {
            "identifier" | "field_identifier" | "property_identifier" => {
                self.node_name(node, source).unwrap_or_default()
            }
            "field_expression" | "selector_expression" | "attribute" | "member_expression" => {
                // For qualified calls like `pkg::func()`, `obj.method()`, or
                // `self.helper()`, the final child is the actual callee name.
                let last = node.child(node.child_count().saturating_sub(1));
                last.map(|n| self.extract_callee_name(n, source))
                    .unwrap_or_default()
            }
            "qualified_identifier" | "scoped_identifier" => {
                // Rust qualified path: take the last identifier segment.
                let last = node.child(node.child_count().saturating_sub(1));
                last.map(|n| self.extract_callee_name(n, source))
                    .unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// Parse a Rust `use` statement into a dot-separated path.
    /// Returns None for complex patterns (e.g. `use foo::{bar, baz}`).
    fn parse_use_path(&self, node: Node, source: &str) -> Option<String> {
        // A use_item has children like: "use" keyword, then the path.
        // The path is typically a single `use_path` child.
        let use_path = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "use_path" || c.kind() == "scoped_use_path");
        let mut parts = Vec::new();
        if let Some(path_node) = use_path {
            // Collect identifier children in order.
            for child in path_node.children(&mut path_node.walk()) {
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    if let Some(text) = self.node_name(child, source) {
                        parts.push(text);
                    }
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("."));
        }
        // Fallback: current tree-sitter-rust wraps the path in a
        // `scoped_identifier` instead of `use_path`; slice it out of the
        // raw statement text ("use crate::util;" -> "crate.util").
        // Complex patterns (globs, brace groups) are still rejected.
        let text = self.node_text(node, source);
        let trimmed = text
            .trim()
            .trim_start_matches("use ")
            .trim_end()
            .trim_end_matches(';')
            .trim();
        if trimmed.is_empty() || trimmed.contains('{') || trimmed.contains('*') {
            return None;
        }
        Some(trimmed.replace("::", "."))
    }

    /// Parse a Go import declaration into a path + optional alias.
    fn parse_go_import(&self, node: Node, source: &str) -> Option<(String, Option<String>)> {
        // An import_declaration contains one or more import_spec children.
        // We only handle single-spec imports here (not grouped blocks).
        let spec = node.child(0)?;
        if spec.kind() != "import_spec" {
            return None;
        }
        // import_spec: [alias?] basic_lit
        let mut alias: Option<String> = None;
        let mut path = String::new();
        for child in spec.children(&mut spec.walk()) {
            match child.kind() {
                "identifier" => {
                    if let Some(name) = self.node_name(child, source) {
                        alias = Some(name);
                    }
                }
                "basic_lit" => {
                    if let Some(text) = self.node_name(child, source) {
                        // Strip surrounding quotes.
                        path = text.trim_matches('"').to_string();
                    }
                }
                _ => {}
            }
        }
        if path.is_empty() {
            None
        } else {
            Some((path, alias))
        }
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

#[cfg(test)]
mod receiver_tests {
    use super::*;

    #[test]
    fn extract_receiver_types_for_path_self_and_plain_calls() {
        let mut p = CodeParser::new("rust").unwrap();
        let src = "struct A;\nimpl A { fn go(&self) { self.save(); Self::prep(); } }\nfn main() { A::new(); helper(); obj.method(); }\n";
        let r = p.parse_file(std::path::Path::new("lib.rs"), src).unwrap();
        let find = |name: &str| {
            r.calls
                .iter()
                .find(|c| c.callee_name == name)
                .unwrap_or_else(|| panic!("no call to {name}"))
        };
        assert_eq!(find("save").receiver_type.as_deref(), Some("self"));
        assert_eq!(find("prep").receiver_type.as_deref(), Some("Self"));
        assert_eq!(find("new").receiver_type.as_deref(), Some("A"));
        assert_eq!(find("helper").receiver_type, None);
        // Plain variable receivers carry no trustworthy type.
        assert_eq!(find("method").receiver_type, None);
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    fn rust_symbols(src: &str) -> Vec<ParsedSymbol> {
        let mut p = CodeParser::new("rust").unwrap();
        p.parse_file(std::path::Path::new("lib.rs"), src)
            .unwrap()
            .symbols
    }

    #[test]
    fn rust_visibility_maps_to_indexer_vocabulary() {
        let syms = rust_symbols(
            "pub fn open_fn() {}\npub(crate) fn crate_fn() {}\npub(super) fn super_fn() {}\nfn private_fn() {}\n",
        );
        let vis = |name: &str| {
            syms.iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("missing {name}"))
                .visibility
                .clone()
        };
        assert_eq!(vis("open_fn").as_deref(), Some("pub"));
        assert_eq!(vis("crate_fn").as_deref(), Some("pub(crate)"));
        assert_eq!(vis("super_fn").as_deref(), Some("pub(super)"));
        // No modifier node: stays unknown rather than misclassified.
        assert_eq!(vis("private_fn"), None);
    }

    #[test]
    fn pub_crate_does_not_shadow_pub() {
        let syms = rust_symbols("pub(crate) struct Inner;\npub struct Outer;\n");
        let vis = |name: &str| {
            syms.iter()
                .find(|s| s.name == name)
                .unwrap()
                .visibility
                .clone()
        };
        assert_eq!(vis("Inner").as_deref(), Some("pub(crate)"));
        assert_eq!(vis("Outer").as_deref(), Some("pub"));
    }

    #[test]
    fn rust_test_attributes_mark_functions() {
        let syms = rust_symbols(
            "#[test]\nfn marked() {}\n#[tokio::test]\nasync fn tokio_marked() {}\nfn plain() {}\n",
        );
        let is_test = |name: &str| syms.iter().find(|s| s.name == name).unwrap().is_test;
        assert!(is_test("marked"));
        assert!(is_test("tokio_marked"));
        assert!(!is_test("plain"));
    }

    #[test]
    fn go_exported_test_functions_are_marked() {
        let mut p = CodeParser::new("go").unwrap();
        let r = p
            .parse_file(
                std::path::Path::new("main_test.go"),
                "func TestBreaker(t *testing.T) {}\nfunc helper() {}\n",
            )
            .unwrap();
        let is_test = |name: &str| r.symbols.iter().find(|s| s.name == name).unwrap().is_test;
        assert!(is_test("TestBreaker"));
        assert!(!is_test("helper"));
    }
}

#[cfg(test)]
mod cross_language_call_tests {
    use super::*;

    fn parse(lang: &str, file: &str, src: &str) -> ParseResult {
        CodeParser::new(lang)
            .unwrap()
            .parse_file(std::path::Path::new(file), src)
            .unwrap()
    }

    fn call<'a>(r: &'a ParseResult, callee: &str) -> &'a ParseCall {
        r.calls
            .iter()
            .find(|c| c.callee_name == callee)
            .unwrap_or_else(|| panic!("no call to {callee} in {:?}", r.calls))
    }

    #[test]
    fn python_calls_and_receivers() {
        let r = parse(
            "python",
            "mod.py",
            "\
class Greeter:
    def greet(self):
        self.hello()
        helper()
        mod.util()
",
        );
        assert_eq!(call(&r, "hello").receiver_type.as_deref(), Some("self"));
        assert!(call(&r, "hello").is_qualified);
        // Caller attribution reaches into methods.
        assert_eq!(call(&r, "hello").caller_symbol.as_deref(), Some("greet"));
        assert_eq!(call(&r, "helper").receiver_type, None);
        assert!(!call(&r, "helper").is_qualified);
        // Plain variable receivers carry no type hint.
        assert_eq!(call(&r, "util").receiver_type, None);
        assert!(call(&r, "util").is_qualified);
    }

    #[test]
    fn python_import_targets() {
        let r = parse(
            "python",
            "mod.py",
            "\
import a.b.c
import x as y
from .helpers import util, other
from pkg import thing
",
        );
        let paths: Vec<&str> = r.import_targets.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"a.b.c"), "{paths:?}");
        let aliased = r.import_targets.iter().find(|i| i.alias.is_some()).unwrap();
        assert_eq!(aliased.path, "x");
        assert_eq!(aliased.alias.as_deref(), Some("y"));
        // `from` imports resolve the MODULE (first dotted name only).
        assert!(paths.contains(&".helpers"), "{paths:?}");
        assert!(paths.contains(&"pkg"), "{paths:?}");
        assert!(!paths.contains(&"pkg.thing"), "{paths:?}");
    }

    #[test]
    fn javascript_calls_constructors_and_import_sources() {
        let r = parse(
            "javascript",
            "app.js",
            "\
import { util } from './utils.js';
import react from 'react';
export { x } from '../shared/helpers.js';

function run(obj) {
    obj.method();
    this.setup();
    helper();
    const u = new User();
    const n = new ns.Widget();
}
",
        );
        assert_eq!(call(&r, "method").receiver_type, None);
        assert!(call(&r, "method").is_qualified);
        assert_eq!(call(&r, "setup").receiver_type.as_deref(), Some("this"));
        assert_eq!(call(&r, "User").callee_name, "User");
        assert_eq!(call(&r, "Widget").callee_name, "Widget");
        assert!(call(&r, "Widget").is_qualified);
        let paths: Vec<&str> = r.import_targets.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"utils.js"), "{paths:?}");
        assert!(paths.contains(&"../shared/helpers.js"), "{paths:?}");
        // Bare external specifier is excluded from structured targets.
        assert!(!paths.contains(&"react"), "{paths:?}");
    }

    #[test]
    fn typescript_calls_route_through_the_shared_extractor() {
        let r = parse(
            "typescript",
            "svc.ts",
            "\
import { db } from './db';
interface Repo { find(): void }
class Svc implements Repo {
    find(): void {
        this.query();
        db.query();
        load();
    }
}
",
        );
        assert_eq!(call(&r, "query").receiver_type.as_deref(), Some("this"));
        assert_eq!(call(&r, "query").caller_symbol.as_deref(), Some("find"));
        assert!(!call(&r, "load").is_qualified);
        let paths: Vec<&str> = r.import_targets.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"db"), "{paths:?}");
    }
}

#[cfg(test)]
mod let_binding_inference_tests {
    use super::*;

    fn call<'a>(r: &'a ParseResult, callee: &str) -> &'a ParseCall {
        r.calls
            .iter()
            .find(|c| c.callee_name == callee)
            .unwrap_or_else(|| panic!("no call to {callee} in {:?}", r.calls))
    }

    #[test]
    fn rust_let_bound_receiver_resolves_to_type() {
        let mut p = CodeParser::new("rust").unwrap();
        let r = p
            .parse_file(
                std::path::Path::new("lib.rs"),
                "fn go() {\n    let p = User::new();\n    p.save();\n    other();\n}\n",
            )
            .unwrap();
        assert_eq!(call(&r, "save").receiver_type.as_deref(), Some("User"));
        assert_eq!(call(&r, "save").caller_symbol.as_deref(), Some("go"));
    }

    #[test]
    fn bindings_do_not_leak_across_functions() {
        let mut p = CodeParser::new("rust").unwrap();
        let r = p
            .parse_file(
                std::path::Path::new("lib.rs"),
                "fn a() {\n    let p = User::new();\n}\nfn b() {\n    p.save();\n}\n",
            )
            .unwrap();
        assert_eq!(call(&r, "save").receiver_type, None);
    }

    #[test]
    fn non_constructor_initializers_stay_untyped() {
        let mut p = CodeParser::new("rust").unwrap();
        let r = p
            .parse_file(
                std::path::Path::new("lib.rs"),
                "fn go() -> u32 {\n    let n = make_thing();\n    n.save();\n    1\n}\n",
            )
            .unwrap();
        assert_eq!(call(&r, "save").receiver_type, None);
    }

    #[test]
    fn python_constructor_assignment_binds_by_convention() {
        let r = CodeParser::new("python")
            .unwrap()
            .parse_file(
                std::path::Path::new("mod.py"),
                "def go():\n    p = Parser()\n    p.parse()\n    q = helper()\n    q.run()\n",
            )
            .unwrap();
        assert_eq!(call(&r, "parse").receiver_type.as_deref(), Some("Parser"));
        // Lowercase callee is a plain function — no binding invented.
        assert_eq!(call(&r, "run").receiver_type, None);
    }

    #[test]
    fn js_new_expression_binding_and_ts_annotation() {
        let js = CodeParser::new("javascript")
            .unwrap()
            .parse_file(
                std::path::Path::new("app.js"),
                "function go() {\n  const w = new Widget();\n  w.render();\n}\n",
            )
            .unwrap();
        assert_eq!(call(&js, "render").receiver_type.as_deref(), Some("Widget"));

        let ts = CodeParser::new("typescript")
            .unwrap()
            .parse_file(
                std::path::Path::new("svc.ts"),
                "function go(u: unknown) {\n  let s: Store = load();\n  s.open();\n}\n",
            )
            .unwrap();
        assert_eq!(call(&ts, "open").receiver_type.as_deref(), Some("Store"));
    }
}

#[cfg(test)]
mod macro_text_recovery_tests {
    use super::*;

    #[test]
    fn calls_inside_macros_are_recovered_and_flagged() {
        let mut p = CodeParser::new("rust").unwrap();
        let r = p
            .parse_file(
                std::path::Path::new("lib.rs"),
                "#[test]\nfn adds() {\n    assert_eq!(add(2, 3), 5);\n    assert_eq!(Ping::probe(), 1);\n}\n",
            )
            .unwrap();
        let add = r
            .calls
            .iter()
            .find(|c| c.callee_name == "add")
            .expect("macro-wrapped call recovered");
        assert!(add.from_macro_text);
        assert!(!add.is_qualified);
        assert_eq!(add.caller_symbol.as_deref(), Some("adds"));
        let probe = r
            .calls
            .iter()
            .find(|c| c.callee_name == "probe")
            .expect("qualified macro call recovered");
        assert!(probe.from_macro_text);
        assert!(probe.is_qualified);
        assert_eq!(probe.receiver_type.as_deref(), Some("Ping"));
        // assert_eq! itself must not be recorded as a callee.
        assert!(r.calls.iter().all(|c| c.callee_name != "assert_eq"));
    }
}

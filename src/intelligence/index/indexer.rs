#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use crate::intelligence::index::database::SymbolDatabase;
use crate::intelligence::index::symbol::{Symbol, SymbolKind, SymbolRelationship};
use crate::intelligence::parser::tree_sitter::{
    CodeParser as TreeSitterParser, SymbolKind as ParserSymbolKind,
};

pub struct CodeIndexer {
    db: SymbolDatabase,
    indexed_files: HashMap<String, String>,
    db_path: PathBuf,
}

impl Clone for CodeIndexer {
    fn clone(&self) -> Self {
        CodeIndexer {
            db: SymbolDatabase::open(&self.db_path).unwrap(),
            indexed_files: self.indexed_files.clone(),
            db_path: self.db_path.clone(),
        }
    }
}

impl CodeIndexer {
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let db = SymbolDatabase::open(&db_path)?;

        Ok(CodeIndexer {
            db,
            indexed_files: HashMap::new(),
            db_path,
        })
    }

    pub fn index_file(&mut self, file_path: &Path, source: &str) -> Result<Vec<Symbol>> {
        let path_str = file_path.to_string_lossy().to_string();

        let language = self.detect_language(file_path);

        if language.is_empty() {
            return Ok(Vec::new());
        }

        let mut parser = TreeSitterParser::new(&language)
            .with_context(|| format!("Failed to create parser for {}", language))?;

        let parse_result = parser.parse_source(source, &path_str)?;

        self.db.delete_symbols_by_file(&path_str)?;

        let mut symbols = Vec::new();
        for ps in &parse_result.symbols {
            let symbol = Symbol {
                id: None,
                name: ps.name.clone(),
                kind: self.map_symbol_kind(&ps.kind),
                language: ps.language.clone(),
                file: path_str.clone(),
                line_start: ps.line_start,
                line_end: ps.line_end,
                column_start: ps.column_start,
                column_end: ps.column_end,
                parent: ps.parent.clone(),
                visibility: ps.visibility.clone(),
                signature: ps.signature.clone(),
                doc_comment: ps.doc_comment.clone(),
            };
            self.db.insert_symbol(&symbol)?;
            symbols.push(symbol);
        }

        for import in &parse_result.imports {
            self.record_import(&path_str, import)?;
        }

        self.indexed_files.insert(path_str, language);

        Ok(symbols)
    }

    pub fn index_directory(&mut self, root: &Path) -> Result<Vec<Symbol>> {
        let mut all_symbols = Vec::new();

        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_dir() {
                continue;
            }

            if self.is_ignored(path, root) {
                continue;
            }

            if let Ok(source) = std::fs::read_to_string(path) {
                let symbols = self.index_file(path, &source)?;
                all_symbols.extend(symbols);
            }
        }

        Ok(all_symbols)
    }

    pub fn incremental_update(&mut self, file_path: &Path, source: &str) -> Result<Vec<Symbol>> {
        self.index_file(file_path, source)
    }

    pub fn remove_file(&mut self, file_path: &Path) -> Result<()> {
        let path_str = file_path.to_string_lossy().to_string();
        self.db.delete_symbols_by_file(&path_str)?;
        self.indexed_files.remove(&path_str);
        Ok(())
    }

    pub fn get_symbols(&self) -> Result<Vec<Symbol>> {
        self.db.get_all_symbols()
    }

    pub fn find_symbol(&self, name: &str) -> Result<Option<Symbol>> {
        self.db.get_symbol_by_name(name)
    }

    pub fn find_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>> {
        self.db.get_symbols_by_file(file)
    }

    pub fn find_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>> {
        self.db.get_symbols_by_kind(kind)
    }

    pub fn find_symbols_by_language(&self, language: &str) -> Result<Vec<Symbol>> {
        self.db.get_symbols_by_language(language)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Symbol>> {
        self.db.search_symbols(query)
    }

    pub fn get_relationships(&self, symbol_name: &str) -> Result<Vec<SymbolRelationship>> {
        self.db.get_relationships_for_symbol(symbol_name)
    }

    pub fn get_dependencies(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        self.db.get_dependencies_for_file(file)
    }

    pub fn get_dependents(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        self.db.get_dependents_of_file(file)
    }

    pub fn clear(&mut self) -> Result<()> {
        self.db.delete_all_symbols()?;
        self.indexed_files.clear();
        Ok(())
    }

    pub fn get_symbol_count(&self) -> Result<u32> {
        self.db.get_symbol_count()
    }

    fn detect_language(&self, path: &Path) -> String {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "rs" => "rust".to_string(),
            "py" => "python".to_string(),
            "js" => "javascript".to_string(),
            "ts" => "typescript".to_string(),
            "tsx" => "tsx".to_string(),
            "jsx" => "jsx".to_string(),
            "go" => "go".to_string(),
            _ => String::new(),
        }
    }

    fn is_ignored(&self, path: &Path, root: &Path) -> bool {
        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_str = relative.to_string_lossy().to_string();

        let ignored_patterns = vec![
            "target/",
            "node_modules/",
            "dist/",
            "build/",
            "vendor/",
            ".git/",
            ".codebro/",
            "*.rs.bk",
            "*.swp",
            "*.swo",
            "*~",
            "*.tmp",
            ".DS_Store",
            "__pycache__/",
            "*.pyc",
            ".pytest_cache/",
            ".venv/",
            "venv/",
            ".mypy_cache/",
            ".ruff_cache/",
            "Cargo.lock",
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
        ];

        for pattern in &ignored_patterns {
            if pattern.ends_with('/') {
                let dir_pattern = pattern.trim_end_matches('/');
                if relative_str.starts_with(dir_pattern) || relative_str == dir_pattern {
                    return true;
                }
            } else if pattern.starts_with('*') {
                if let Some(ext) = pattern.strip_prefix("*") {
                    if relative_str.ends_with(ext) {
                        return true;
                    }
                }
            } else if relative_str == *pattern {
                return true;
            }
        }

        false
    }

    fn map_symbol_kind(&self, kind: &ParserSymbolKind) -> SymbolKind {
        use crate::intelligence::index::symbol::SymbolKind as ISK;

        match kind {
            ParserSymbolKind::Function => ISK::Function,
            ParserSymbolKind::Class => ISK::Class,
            ParserSymbolKind::Struct => ISK::Struct,
            ParserSymbolKind::Enum => ISK::Enum,
            ParserSymbolKind::Trait => ISK::Trait,
            ParserSymbolKind::Interface => ISK::Interface,
            ParserSymbolKind::Method => ISK::Method,
            ParserSymbolKind::Variable => ISK::Variable,
            ParserSymbolKind::Constant => ISK::Constant,
            ParserSymbolKind::TypeAlias => ISK::TypeAlias,
            ParserSymbolKind::Module => ISK::Module,
            ParserSymbolKind::Import => ISK::Import,
            ParserSymbolKind::Export => ISK::Export,
            ParserSymbolKind::Field => ISK::Field,
            ParserSymbolKind::Parameter => ISK::Parameter,
            ParserSymbolKind::Macro => ISK::Macro,
            ParserSymbolKind::Impl => ISK::Impl,
            ParserSymbolKind::Constructor => ISK::Constructor,
        }
    }

    fn record_import(&self, file: &str, import_text: &str) -> Result<()> {
        let imported = self.extract_imported_symbol(import_text);
        if let Some(imported_symbol) = imported {
            let relationship = SymbolRelationship {
                from_symbol: String::new(),
                from_file: file.to_string(),
                to_symbol: imported_symbol,
                to_file: String::new(),
                relationship_type: "imports".to_string(),
            };
            self.db.insert_relationship(&relationship)?;
        }
        Ok(())
    }

    fn extract_imported_symbol(&self, import_text: &str) -> Option<String> {
        let text = import_text.trim();

        if text.starts_with("use ") && text.ends_with(';') {
            let inner = &text[4..text.len() - 1];
            let parts: Vec<&str> = inner.split("::").collect();
            if let Some(last) = parts.last() {
                return Some(last.trim().to_string());
            }
        }

        if text.starts_with("import ") {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if parts.len() >= 2 {
                let path = parts[1].trim_end_matches(';');
                let module_name = path.rsplit('.').next().unwrap_or(path);
                return Some(module_name.to_string());
            }
        }

        if text.starts_with("from ") {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if parts.len() >= 4 && parts[2] == "import" {
                let path = parts[1];
                let module_name = path.rsplit('.').next().unwrap_or(path);
                return Some(module_name.to_string());
            }
        }

        if text.starts_with("require(") || text.starts_with("import(") {
            let start = text.find('"').or_else(|| text.find('\''))?;
            let end = text[start + 1..].find(|c: char| c == '"' || c == '\'')?;
            let path = &text[start + 1..start + 1 + end];
            let module_name = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(path);
            return Some(module_name.to_string());
        }

        None
    }
}

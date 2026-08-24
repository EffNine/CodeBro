#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use crate::intelligence::index::symbol::{Symbol, SymbolRelationship};

pub struct SymbolDatabase {
    conn: Connection,
}

impl SymbolDatabase {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open database at {:?}", path.as_ref()))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                language TEXT NOT NULL,
                file TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                column_start INTEGER NOT NULL,
                column_end INTEGER NOT NULL,
                parent TEXT,
                visibility TEXT,
                signature TEXT,
                doc_comment TEXT
            )",
            [],
        )
        .context("Failed to create symbols table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_symbol TEXT NOT NULL,
                from_file TEXT NOT NULL,
                to_symbol TEXT NOT NULL,
                to_file TEXT NOT NULL,
                relationship_type TEXT NOT NULL
            )",
            [],
        )
        .context("Failed to create relationships table")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name)",
            [],
        )
        .context("Failed to create index on symbols.name")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file)",
            [],
        )
        .context("Failed to create index on symbols.file")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind)",
            [],
        )
        .context("Failed to create index on symbols.kind")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbols_language ON symbols(language)",
            [],
        )
        .context("Failed to create index on symbols.language")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_relationships_from ON relationships(from_symbol, from_file)",
            [],
        )
        .context("Failed to create index on relationships.from")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_relationships_to ON relationships(to_symbol, to_file)",
            [],
        )
        .context("Failed to create index on relationships.to")?;

        Ok(SymbolDatabase { conn })
    }

    pub fn insert_symbol(&self, symbol: &Symbol) -> Result<i64> {
        let kind_str = format!("{}", symbol.kind);

        let _id = self.conn.last_insert_rowid();

        self.conn.execute(
            "INSERT INTO symbols (name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                symbol.name,
                kind_str,
                symbol.language,
                symbol.file,
                symbol.line_start,
                symbol.line_end,
                symbol.column_start,
                symbol.column_end,
                symbol.parent,
                symbol.visibility,
                symbol.signature,
                symbol.doc_comment,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_symbols(&self, symbols: &[Symbol]) -> Result<()> {
        for symbol in symbols {
            self.insert_symbol(symbol)?;
        }
        Ok(())
    }

    pub fn get_symbol_by_name(&self, name: &str) -> Result<Option<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols WHERE name = ?1",
        )?;

        let symbol = stmt
            .query_row(params![name], |row| Self::row_to_symbol(row))
            .optional()?;

        Ok(symbol)
    }

    pub fn get_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols WHERE file = ?1 ORDER BY line_start",
        )?;

        let symbols = stmt
            .query_map(params![file], |row| Self::row_to_symbol(row))?
            .filter_map(|s| s.ok())
            .collect();

        Ok(symbols)
    }

    pub fn get_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols WHERE kind = ?1 ORDER BY name",
        )?;

        let symbols = stmt
            .query_map(params![kind], |row| Self::row_to_symbol(row))?
            .filter_map(|s| s.ok())
            .collect();

        Ok(symbols)
    }

    pub fn get_symbols_by_language(&self, language: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols WHERE language = ?1 ORDER BY file, line_start",
        )?;

        let symbols = stmt
            .query_map(params![language], |row| Self::row_to_symbol(row))?
            .filter_map(|s| s.ok())
            .collect();

        Ok(symbols)
    }

    pub fn search_symbols(&self, query: &str) -> Result<Vec<Symbol>> {
        let like_query = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols WHERE name LIKE ?1 OR signature LIKE ?1 OR doc_comment LIKE ?1
             ORDER BY name",
        )?;

        let symbols = stmt
            .query_map(params![like_query], |row| Self::row_to_symbol(row))?
            .filter_map(|s| s.ok())
            .collect();

        Ok(symbols)
    }

    pub fn get_all_symbols(&self) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols ORDER BY file, line_start",
        )?;

        let symbols = stmt
            .query_map([], |row| Self::row_to_symbol(row))?
            .filter_map(|s| s.ok())
            .collect();

        Ok(symbols)
    }

    pub fn get_symbol_count(&self) -> Result<u32> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        Ok(count as u32)
    }

    /// Distinct indexed file paths, sorted for determinism.
    pub fn list_indexed_files(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT file FROM symbols ORDER BY file")?;
        let files = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|f| f.ok())
            .collect();
        Ok(files)
    }

    pub fn delete_symbols_by_file(&self, file: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM symbols WHERE file = ?1", params![file])?;
        Ok(())
    }

    pub fn delete_all_symbols(&self) -> Result<()> {
        self.conn.execute("DELETE FROM symbols", [])?;
        self.conn.execute("DELETE FROM relationships", [])?;
        Ok(())
    }

    pub fn insert_relationship(&self, relationship: &SymbolRelationship) -> Result<()> {
        self.conn.execute(
            "INSERT INTO relationships (from_symbol, from_file, to_symbol, to_file, relationship_type)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                relationship.from_symbol,
                relationship.from_file,
                relationship.to_symbol,
                relationship.to_file,
                relationship.relationship_type,
            ],
        )?;
        Ok(())
    }

    pub fn get_relationships_for_symbol(
        &self,
        symbol_name: &str,
    ) -> Result<Vec<SymbolRelationship>> {
        let mut stmt = self.conn.prepare(
            "SELECT from_symbol, from_file, to_symbol, to_file, relationship_type
             FROM relationships WHERE from_symbol = ?1 OR to_symbol = ?1",
        )?;

        let relationships = stmt
            .query_map(params![symbol_name], |row| {
                Ok(SymbolRelationship {
                    from_symbol: row.get(0)?,
                    from_file: row.get(1)?,
                    to_symbol: row.get(2)?,
                    to_file: row.get(3)?,
                    relationship_type: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(relationships)
    }

    pub fn get_dependencies_for_file(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        let mut stmt = self.conn.prepare(
            "SELECT from_symbol, from_file, to_symbol, to_file, relationship_type
             FROM relationships WHERE from_file = ?1",
        )?;

        let relationships = stmt
            .query_map(params![file], |row| {
                Ok(SymbolRelationship {
                    from_symbol: row.get(0)?,
                    from_file: row.get(1)?,
                    to_symbol: row.get(2)?,
                    to_file: row.get(3)?,
                    relationship_type: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(relationships)
    }

    pub fn get_dependents_of_file(&self, file: &str) -> Result<Vec<SymbolRelationship>> {
        let mut stmt = self.conn.prepare(
            "SELECT from_symbol, from_file, to_symbol, to_file, relationship_type
             FROM relationships WHERE to_file = ?1",
        )?;

        let relationships = stmt
            .query_map(params![file], |row| {
                Ok(SymbolRelationship {
                    from_symbol: row.get(0)?,
                    from_file: row.get(1)?,
                    to_symbol: row.get(2)?,
                    to_file: row.get(3)?,
                    relationship_type: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(relationships)
    }

    pub fn get_symbols_containing_text(&self, text: &str) -> Result<Vec<Symbol>> {
        let like_query = format!("%{}%", text);
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file, line_start, line_end, column_start, column_end, parent, visibility, signature, doc_comment
             FROM symbols WHERE name LIKE ?1 OR signature LIKE ?1",
        )?;

        let symbols = stmt
            .query_map(params![like_query], |row| Self::row_to_symbol(row))?
            .filter_map(|s| s.ok())
            .collect();

        Ok(symbols)
    }

    fn row_to_symbol(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
        let kind_str: String = row.get(2)?;
        let kind = match kind_str.as_str() {
            "function" => crate::intelligence::index::symbol::SymbolKind::Function,
            "class" => crate::intelligence::index::symbol::SymbolKind::Class,
            "struct" => crate::intelligence::index::symbol::SymbolKind::Struct,
            "enum" => crate::intelligence::index::symbol::SymbolKind::Enum,
            "trait" => crate::intelligence::index::symbol::SymbolKind::Trait,
            "interface" => crate::intelligence::index::symbol::SymbolKind::Interface,
            "method" => crate::intelligence::index::symbol::SymbolKind::Method,
            "variable" => crate::intelligence::index::symbol::SymbolKind::Variable,
            "constant" => crate::intelligence::index::symbol::SymbolKind::Constant,
            "type_alias" => crate::intelligence::index::symbol::SymbolKind::TypeAlias,
            "module" => crate::intelligence::index::symbol::SymbolKind::Module,
            "import" => crate::intelligence::index::symbol::SymbolKind::Import,
            "export" => crate::intelligence::index::symbol::SymbolKind::Export,
            "field" => crate::intelligence::index::symbol::SymbolKind::Field,
            "parameter" => crate::intelligence::index::symbol::SymbolKind::Parameter,
            "macro" => crate::intelligence::index::symbol::SymbolKind::Macro,
            "impl" => crate::intelligence::index::symbol::SymbolKind::Impl,
            "constructor" => crate::intelligence::index::symbol::SymbolKind::Constructor,
            _ => crate::intelligence::index::symbol::SymbolKind::Function,
        };

        Ok(Symbol {
            id: Some(row.get(0)?),
            name: row.get(1)?,
            kind,
            language: row.get(3)?,
            file: row.get(4)?,
            line_start: row.get(5)?,
            line_end: row.get(6)?,
            column_start: row.get(7)?,
            column_end: row.get(8)?,
            parent: row.get(9)?,
            visibility: row.get(10)?,
            signature: row.get(11)?,
            doc_comment: row.get(12)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intelligence::index::symbol::Symbol;
    use crate::intelligence::index::symbol::SymbolKind;

    fn symbol(name: &str, file: &str) -> Symbol {
        Symbol {
            id: None,
            name: name.to_string(),
            kind: SymbolKind::Function,
            language: "rust".to_string(),
            file: file.to_string(),
            line_start: 1,
            line_end: 1,
            column_start: 0,
            column_end: 0,
            parent: None,
            visibility: None,
            signature: None,
            doc_comment: None,
        }
    }

    #[test]
    fn test_list_indexed_files_distinct_and_sorted() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let db = SymbolDatabase::open(tmp.path().join("index.db")).expect("open");
        db.insert_symbol(&symbol("a", "z.rs")).unwrap();
        db.insert_symbol(&symbol("b", "a.rs")).unwrap();
        db.insert_symbol(&symbol("c", "z.rs")).unwrap();
        db.insert_symbol(&symbol("d", "m.rs")).unwrap();

        let files = db.list_indexed_files().expect("list");
        assert_eq!(files, vec!["a.rs", "m.rs", "z.rs"]);
    }

    #[test]
    fn test_list_indexed_files_empty() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let db = SymbolDatabase::open(tmp.path().join("index.db")).expect("open");
        assert!(db.list_indexed_files().expect("list").is_empty());
    }
}

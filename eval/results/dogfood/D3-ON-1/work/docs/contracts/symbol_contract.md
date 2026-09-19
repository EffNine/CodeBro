# Symbol Contract

**Version:** 1.0.0
**Status:** Accepted
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

This contract defines the symbol data model used throughout the intelligence platform. Symbols are the fundamental unit of code understanding.

---

## 2. SymbolKind Enum

```rust
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
```

### 2.1 Kind Semantics

| Kind | Description |
|------|-------------|
| `Function` | Free-standing function definition |
| `Class` | Class declaration |
| `Struct` | Structure declaration |
| `Enum` | Enumeration declaration |
| `Trait` | Trait definition (Rust) / Interface (TS/JS) |
| `Interface` | Interface declaration |
| `Method` | Method inside a class/struct/trait |
| `Variable` | Variable binding |
| `Constant` | Constant binding |
| `TypeAlias` | Type alias declaration |
| `Module` | Module declaration |
| `Import` | Import statement |
| `Export` | Export statement |
| `Field` | Struct/enum field |
| `Parameter` | Function parameter |
| `Macro` | Macro definition |
| `Impl` | Trait implementation block |
| `Constructor` | Constructor method |

---

## 3. Symbol Structure

```rust
pub struct Symbol {
    pub id: Option<i64>,
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
```

### 3.1 Field Contracts

| Field | Required | Constraint |
|-------|----------|------------|
| `id` | No | Auto-assigned by database; `None` for new symbols |
| `name` | Yes | Non-empty, unique within file |
| `kind` | Yes | Must be a valid `SymbolKind` |
| `language` | Yes | Must match a supported language |
| `file` | Yes | Absolute or project-relative path |
| `line_start` | Yes | 1-indexed, >= 1 |
| `line_end` | Yes | >= line_start |
| `column_start` | Yes | 0-indexed |
| `column_end` | Yes | >= column_start |
| `parent` | No | Name of parent symbol (for nested symbols) |
| `visibility` | No | "public", "crate", "private", or `None` |
| `signature` | No | Function/method signature text |
| `doc_comment` | No | Documentation comment text |

---

## 4. SymbolRelationship Structure

```rust
pub struct SymbolRelationship {
    pub from_symbol: String,
    pub from_file: String,
    pub to_symbol: String,
    pub to_file: String,
    pub relationship_type: String,
}
```

### 4.1 Relationship Types

| Type | Description |
|------|-------------|
| `imports` | From-file imports from-to-symbol |
| `calls` | From-symbol calls to-symbol |
| `extends` | From-symbol extends to-symbol (inheritance) |
| `implements` | From-symbol implements to-symbol (interface) |
| `references` | From-symbol references to-symbol |

---

## 5. FileInfo Structure

```rust
pub struct FileInfo {
    pub path: String,
    pub language: String,
    pub symbol_count: u32,
    pub last_indexed: String,
}
```

---

## 6. Symbol Database Contract

### 6.1 Schema

```sql
CREATE TABLE symbols (
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
);

CREATE TABLE relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_symbol TEXT NOT NULL,
    from_file TEXT NOT NULL,
    to_symbol TEXT NOT NULL,
    to_file TEXT NOT NULL,
    relationship_type TEXT NOT NULL
);
```

### 6.2 Indexes

| Index | Columns | Purpose |
|-------|---------|---------|
| `idx_symbols_name` | `name` | Fast name lookup |
| `idx_symbols_file` | `file` | File-based queries |
| `idx_symbols_kind` | `kind` | Kind-based queries |
| `idx_symbols_language` | `language` | Language-based queries |
| `idx_relationships_from` | `from_symbol, from_file` | Dependency queries |
| `idx_relationships_to` | `to_symbol, to_file` | Dependent queries |

---

## 7. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-05 | Initial contract definition |

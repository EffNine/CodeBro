# Intelligence Platform Contract

**Version:** 1.0.0
**Status:** Accepted
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

This contract defines the intelligence platform's role, boundaries, and interfaces within the CodeBro system. The intelligence platform is a **read-only** analysis layer that provides code understanding capabilities to the agent runtime.

---

## 2. Platform Boundaries

### 2.1 What the Intelligence Platform Does

- Parses source code into abstract syntax trees (ASTs)
- Extracts symbols (functions, classes, types, etc.) from ASTs
- Stores symbols in a persistent database
- Builds dependency graphs from import/relationship data
- Provides semantic search over indexed symbols
- Assembles contextual information for agent reasoning
- Records diagnostics for platform health monitoring

### 2.2 What the Intelligence Platform Does NOT Do

- **Never writes source files** — File modification is the tool layer's responsibility
- **Never executes commands** — Command execution is the tool layer's responsibility
- **Never makes LLM calls** — LLM interaction is the provider layer's responsibility
- **Never modifies agent state** — Agent state is managed by the agent layer
- **Never stores user preferences** — User configuration is managed by the config layer

---

## 3. Public Interface

### 3.1 Module Exports

```rust
// src/intelligence/mod.rs
pub mod parser;    // Code parsing
pub mod index;     // Symbol indexing
pub mod graph;     // Dependency graph
pub mod search;    // Semantic search
pub mod context;   // Context building
pub mod reasoning; // Reasoning engine
pub mod memory;    // Intelligence memory
pub mod lsp;       // LSP abstraction
pub mod diagnostics; // Platform diagnostics
```

### 3.2 Trait Interface

| Trait | Module | Purpose |
|-------|--------|---------|
| `CodeParser` | `parser` | Parse source into symbols |
| `SymbolDatabase` | `index` | Store/retrieve symbols |
| `CodeIndexer` | `index` | Index files and directories |
| `DependencyGraph` | `graph` | Represent code dependencies |
| `SemanticSearch` | `search` | Search indexed symbols |
| `ContextBuilder` | `context` | Build context for agents |
| `ReasoningEngine` | `reasoning` | Analyze code changes |
| ~~`IntelligenceMemory`~~ | ~~`memory`~~ | Removed in ADR-012 — project knowledge is owned by `project_identity`; fact model by `engineering_facts` |
| `LspFoundation` | `lsp` | LSP protocol types |
| `IntelligenceDiagnostics` | `diagnostics` | Platform health monitoring |

---

## 4. Data Contracts

### 4.1 Symbol Contract

Every symbol must have:
- `name: String` — Unique within file
- `kind: SymbolKind` — One of 19 defined kinds
- `file: String` — Absolute or project-relative path
- `line_start, line_end: u32` — 1-indexed line range
- `column_start, column_end: u32` — 0-indexed column range

Optional fields:
- `parent: Option<String>` — Parent symbol name (for nested symbols)
- `visibility: Option<String>` — "public", "crate", "private"
- `signature: Option<String>` — Function/method signature text
- `doc_comment: Option<String>` — Documentation comment

### 4.2 Relationship Contract

Every relationship must have:
- `from_symbol: String` — Source symbol name
- `from_file: String` — Source file path
- `to_symbol: String` — Target symbol name
- `to_file: String` — Target file path
- `relationship_type: String` — "imports", "calls", "extends", etc.

### 4.3 Context Contract

Every `IntelligenceContext` must have:
- `query: String` — The original search query
- `relevant_symbols: Vec<SearchResult>` — Ranked symbol results
- `related_files: Vec<String>` — Files containing relevant symbols
- `dependencies: Vec<String>` — Dependency graph expansion
- `code_snippets: Vec<CodeSnippet>` — Extracted code snippets
- `total_symbols_found: usize` — Total count of relevant symbols

---

## 5. Performance Contracts

| Operation | Target Latency | Notes |
|-----------|---------------|-------|
| Parse single file | < 50ms | For files up to 10,000 lines |
| Index single file | < 100ms | Includes parse + DB write |
| Incremental update | < 50ms | Only for changed files |
| Symbol search | < 10ms | For up to 100,000 symbols |
| Context build | < 200ms | Includes search + graph expansion |
| Graph build | < 500ms | For up to 10,000 files |

---

## 6. Error Contracts

| Error Type | Recovery | Logging |
|------------|----------|---------|
| Parse error | Skip file, continue | Warning |
| DB error | Return error to caller | Error |
| File not found | Skip file, continue | Debug |
| Graph cycle detected | Log warning, continue | Warning |
| Search timeout | Return partial results | Warning |

---

## 7. Threading Contracts

- All public traits are `Send + Sync`
- `CodeIndexer` uses `Arc<Mutex<>>` for SQLite connection sharing
- `IntelligenceDiagnostics` uses `Arc<Mutex<>>` for thread-safe recording
- No shared mutable state between parser instances

---

## 8. Extension Contracts

| Extension Point | How to Extend | ADR Required |
|----------------|---------------|--------------|
| New language parser | Add to `languages.rs`, add extraction in `tree_sitter.rs` | Yes |
| New symbol kind | Add to `SymbolKind` enum | Yes |
| New relationship type | Add to `relationship_type` string values | No |
| New search strategy | Implement `SemanticSearch` trait | No |
| New diagnostics metric | Add to `IntelligenceDiagnostics` trait | No |

---

## 9. Dependencies

| Dependency | Purpose | Version |
|------------|---------|---------|
| `tree-sitter` | AST parsing | 0.20 |
| `tree-sitter-rust` | Rust parsing | 0.20 |
| `tree-sitter-python` | Python parsing | 0.20 |
| `tree-sitter-javascript` | JS parsing | 0.20 |
| `tree-sitter-typescript` | TS/TSX parsing | 0.20 |
| `tree-sitter-go` | Go parsing | 0.20 |
| `rusqlite` | Symbol database | 0.31 |
| `serde` | Serialization | 1 |
| `walkdir` | Directory traversal | 2 |

---

## 10. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-05 | Initial contract definition |

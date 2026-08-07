# Architecture Snapshot v1.0

**Document:** `docs/architecture/architecture_snapshot_v1.md`
**Version:** 1.0.0
**Effective:** P4 Intelligence Platform
**Status:** Baseline for P4
**Supercedes:** N/A (new architecture layer)

---

## 1. Module Graph

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Intelligence Platform                             │
│                            (src/intelligence/)                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐   ┌─────────────┐  │
│  │   Parser     │   │   Indexer    │   │    Symbol    │   │   LSP       │  │
│  │   Platform   │──▶│   Platform   │──▶│   Model      │──▶│  Abstraction│  │
│  │              │   │              │   │              │   │             │  │
│  │ • tree-sitter│   │ • SQLite     │   │ • Symbol     │   │ • Position  │  │
│  │ • Language   │   │ • SymbolDB   │   │ • Kind       │   │ • Range     │  │
│  │ • AST Walk   │   │ • Indexer    │   │ • Relation   │   │ • Hover     │  │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘   └─────────────┘  │
│         │                  │                   │                           │
│         │          ┌───────▼───────────────────▼───────┐                   │
│         │          │     Symbol Database (SQLite)       │                   │
│         │          │   name, kind, file, line,          │                   │
│         │          │   parent, visibility, sig, docs    │                   │
│         │          └───────┬───────────────────┬───────┘                   │
│         │                  │                   │                           │
│         │          ┌───────▼───────┐   ┌───────▼───────┐                    │
│         │          │  Semantic     │   │  Dependency   │                    │
│         │          │  Search       │   │  Graph        │                    │
│         │          │  Interface    │   │  Interface    │                    │
│         │          └───────┬───────┘   └───────┬───────┘                    │
│         │                  │                   │                           │
│         │          ┌───────▼───────────────────▼───────┐                   │
│         │          │     Context Builder               │                   │
│         │          │  • Semantic query → symbols       │                   │
│         │          │  • Graph expansion → deps/imps    │                   │
│         │          │  • Snippet extraction             │                   │
│         │          └───────────────┬───────────────────┘                   │
│         │                          │                                      │
│         │          ┌───────────────▼───────────────────┐                   │
│         │          │     Reasoning Interface           │                   │
│         │          │  • analyze_before_modification    │                   │
│         │          │  • analyze_for_code_understanding │                   │
│         │          │  • find_existing_patterns         │                   │
│         │          └───────────────┬───────────────────┘                   │
│         │                          │                                      │
│         │          ┌───────────────▼───────────────────┐                   │
│         │          │     Intelligence Memory           │                   │
│         │          │  • Project intelligence state     │                   │
│         │          │  • Symbol patterns                │                   │
│         │          │  • Architecture patterns          │                   │
│         │          └───────────────┬───────────────────┘                   │
│         │                          │                                      │
│         │          ┌───────────────▼───────────────────┐                   │
│         │          │     Intelligence Diagnostics      │                   │
│         │          │  • Parse metrics                  │                   │
│         │          │  • Index health                   │                   │
│         │          │  • Graph integrity                │                   │
│         │          │  • Search quality                 │                   │
│         │          │  • Context quality                │                   │
│         │          └───────────────────────────────────┘                   │
│         └─────────────────────────────────────────────────────────────────┘│
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Data Flow:**

```
Source Code (filesystem)
    │
    ▼
Parser Platform (tree-sitter AST)
    │  ParsedSymbol
    ▼
Indexer Platform (SQLite write)
    │  Symbol (persisted)
    ▼
Symbol Database
    ├──► Symbol Model (query/read)
    │       │
    │       ▼
    │   Semantic Search Interface
    │       │
    │       ▼
    │   Context Builder
    │       │
    │       ▼
    │   Reasoning Interface
    │
    ├──► Dependency Graph (built from relationships)
    │       │
    │       ▼
    │   Context Builder (graph expansion)
    │
    ├──► Intelligence Memory (pattern recognition)
    │
    └──► Intelligence Diagnostics (observability)
```

---

## 2. Public Trait Inventory

### 2.1 CodeParser (Parser Platform)

```rust
pub trait CodeParser: Send + Sync {
    fn parse(&mut self, source: &str, file_path: &str) -> Result<ParseResult>;
    fn supported_languages(&self) -> Vec<&str>;
    fn language_name(&self) -> &str;
}
```

**Implementations:**
- `TreeSitterParser` — tree-sitter backed parser

### 2.2 SymbolDatabase (Index Platform)

```rust
pub trait SymbolDatabase: Send + Sync {
    fn insert_symbol(&self, symbol: &Symbol) -> Result<i64>;
    fn insert_symbols(&self, symbols: &[Symbol]) -> Result<()>;
    fn get_symbol_by_name(&self, name: &str) -> Result<Option<Symbol>>;
    fn get_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>>;
    fn get_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>>;
    fn get_symbols_by_language(&self, language: &str) -> Result<Vec<Symbol>>;
    fn search_symbols(&self, query: &str) -> Result<Vec<Symbol>>;
    fn get_all_symbols(&self) -> Result<Vec<Symbol>>;
    fn delete_symbols_by_file(&self, file: &str) -> Result<()>;
    fn delete_all_symbols(&self) -> Result<()>;
    fn get_symbol_count(&self) -> Result<u32>;

    // Relationship queries
    fn insert_relationship(&self, rel: &SymbolRelationship) -> Result<()>;
    fn get_relationships_for_symbol(&self, name: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependencies_for_file(&self, file: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependents_of_file(&self, file: &str) -> Result<Vec<SymbolRelationship>>;
}
```

**Implementations:**
- `SqliteSymbolDatabase` — SQLite-backed implementation

### 2.3 CodeIndexer (Index Platform)

```rust
pub trait CodeIndexer: Send + Sync {
    // File-level indexing
    fn index_file(&mut self, path: &Path, source: &str) -> Result<Vec<Symbol>>;
    fn incremental_update(&mut self, path: &Path, source: &str) -> Result<Vec<Symbol>>;
    fn remove_file(&mut self, path: &Path) -> Result<()>;

    // Directory-level indexing
    fn index_directory(&mut self, root: &Path) -> Result<Vec<Symbol>>;

    // Query interface
    fn get_symbols(&self) -> Result<Vec<Symbol>>;
    fn find_symbol(&self, name: &str) -> Result<Option<Symbol>>;
    fn find_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>>;
    fn find_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>>;
    fn find_symbols_by_language(&self, lang: &str) -> Result<Vec<Symbol>>;
    fn search(&self, query: &str) -> Result<Vec<Symbol>>;
    fn get_symbol_count(&self) -> Result<u32>;

    // Relationship queries
    fn get_relationships(&self, symbol_name: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependencies(&self, file: &str) -> Result<Vec<SymbolRelationship>>;
    fn get_dependents(&self, file: &str) -> Result<Vec<SymbolRelationship>>;

    // Maintenance
    fn clear(&mut self) -> Result<()>;
    fn get_indexed_files(&self) -> Vec<String>;
}
```

**Implementations:**
- `SqliteCodeIndexer` — production implementation

### 2.4 DependencyGraph (Symbol Graph)

```rust
pub trait DependencyGraph: Send + Sync {
    fn new() -> Self;
    fn from_indexer(indexer: &dyn CodeIndexer) -> Result<Self>;

    fn add_node(&mut self, file: String);
    fn add_edge(&mut self, from: String, to: String);

    fn get_dependencies(&self, file: &str) -> Vec<String>;
    fn get_dependents(&self, file: &str) -> Vec<String>;
    fn get_transitive_dependencies(&self, file: &str) -> HashSet<String>;
    fn get_transitive_dependents(&self, file: &str) -> HashSet<String>;
    fn get_all_files(&self) -> Vec<String>;
    fn get_symbol_files(&self, symbol_name: &str) -> Vec<String>;
    fn find_path(&self, from: &str, to: &str) -> Option<Vec<String>>;

    fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()>;
    fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self>;
}
```

### 2.5 SemanticSearch (Search Interface)

```rust
pub trait SemanticSearch: Send + Sync {
    fn new(indexer: CodeIndexer) -> Self;

    fn search(&self, query: &str) -> Result<Vec<SearchResult>>;
    fn search_by_question(&self, question: &str) -> Result<Vec<SearchResult>>;
    fn find_symbol(&self, name: &str) -> Result<Option<Symbol>>;
    fn find_symbols_by_file(&self, file: &str) -> Result<Vec<Symbol>>;
    fn find_symbols_by_kind(&self, kind: &str) -> Result<Vec<Symbol>>;
    fn find_symbols_by_language(&self, lang: &str) -> Result<Vec<Symbol>>;
    fn find_related(&self, symbol_name: &str) -> Result<Vec<SearchResult>>;
}
```

### 2.6 ContextBuilder (Context Interface)

```rust
pub trait ContextBuilder: Send + Sync {
    fn build_context(&self, query: &str) -> Result<IntelligenceContext>;
    fn build_context_for_modification(
        &self,
        target_symbol: &str,
    ) -> Result<IntelligenceContext>;
    fn get_related_symbols(&self, symbol_name: &str) -> Result<Vec<SearchResult>>;
    fn get_symbol_dependencies(&self, symbol_name: &str) -> Result<Vec<String>>;
}
```

### 2.7 ReasoningEngine (Reasoning Interface)

```rust
pub trait ReasoningEngine: Send + Sync {
    fn analyze_before_modification(&self, request: &str) -> Result<ReasoningResult>;
    fn analyze_for_code_understanding(&self, file_path: &str) -> Result<ReasoningResult>;
    fn find_existing_patterns(&self, pattern_name: &str) -> Result<Vec<String>>;
    fn suggest_implementation_approach(&self, request: &str) -> Result<Vec<String>>;
}
```

### 2.8 IntelligenceMemory (Memory Interface)

```rust
pub trait IntelligenceMemory: Send + Sync {
    fn new() -> Result<Self>;
    fn save(&self) -> Result<()>;

    fn record_symbol(&mut self, name: String, kind: String, file: String, reason: String);
    fn record_pattern(&mut self, name: String, description: String, files: Vec<String>, confidence: f32);
    fn record_convention(&mut self, convention: String);
    fn record_relationship(&mut self, from: String, to: String, rel_type: String, file: String);

    fn get_important_symbols(&self) -> &[ImportantSymbol];
    fn get_architecture_patterns(&self) -> &[ArchitecturePattern];
    fn get_conventions(&self) -> &[String];
    fn get_relationships(&self) -> &[DiscoveredRelationship];
    fn get_project_structure(&self) -> Option<&ProjectStructure>;
    fn set_project_structure(&mut self, structure: ProjectStructure);

    fn analyze_project(&mut self, indexer: &dyn CodeIndexer) -> Result<()>;
}
```

### 2.9 LspFoundation (LSP Abstraction)

```rust
pub trait LspFoundation: Send + Sync {
    fn new() -> Self;

    // Document management
    fn open_document(&mut self, doc: LspTextDocumentItem);
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
    fn rename_symbol(&self, uri: &str, pos: &LspPosition, new_name: &str) -> Option<LspWorkspaceEdit>;
}
```

### 2.10 IntelligenceDiagnostics (Diagnostics Interface)

```rust
pub trait IntelligenceDiagnostics: Send + Sync {
    fn new() -> Self;

    // Parse metrics
    fn record_parse(&mut self, file: &str, language: &str, duration_ms: f64, symbol_count: usize, error_count: usize);
    fn get_parse_metrics(&self) -> Vec<ParseMetric>;

    // Index health
    fn record_index_event(&mut self, event: IndexEvent);
    fn get_index_health(&self) -> IndexHealth;

    // Graph integrity
    fn record_graph_event(&mut self, event: GraphEvent);
    fn get_graph_integrity(&self) -> GraphIntegrity;

    // Search quality
    fn record_search(&mut self, query: &str, result_count: usize, duration_ms: f64);
    fn get_search_metrics(&self) -> Vec<SearchMetric>;

    // Context quality
    fn record_context_build(&mut self, query: &str, symbol_count: usize, file_count: usize, duration_ms: f64);
    fn get_context_metrics(&self) -> Vec<ContextMetric>;

    // Summary
    fn summary(&self) -> String;
    fn clear(&mut self);
}
```

---

## 3. Extension Points

### 3.1 Parser Extension

**Point:** `src/intelligence/parser/languages.rs`

New languages are added by:
1. Adding the tree-sitter crate to `Cargo.toml`
2. Adding a variant to `get_language()` in `languages.rs`
3. Adding extraction logic in `tree_sitter.rs`

**Reserved:** Language name prefix `unknown_` is reserved for future custom parsers.

### 3.2 Indexer Extension

**Point:** `src/intelligence/index/database.rs`

New storage backends are added by implementing `SymbolDatabase` trait. The SQLite implementation is the default; in-memory or alternative backends can be swapped.

**Reserved:** Database path prefix `memory://` for in-memory databases.

### 3.3 Search Extension

**Point:** `src/intelligence/search/semantic.rs`

New search strategies can be added by implementing `SemanticSearch` trait. The scoring algorithm is pluggable.

**Reserved:** `MatchType::Embedding` is reserved for future embedding-based search.

### 3.4 Graph Extension

**Point:** `src/intelligence/graph/dependency.rs`

Graph algorithms (cycle detection, SCC, topological sort) can be added as methods on `DependencyGraph`.

**Reserved:** `GraphAlgorithm` enum variant prefix `custom_` for future algorithm extensions.

### 3.5 LSP Extension

**Point:** `src/intelligence/lsp/foundation.rs`

Additional LSP features can be added to `LspFoundation`. The trait defines the contract; implementations provide the behavior.

**Reserved:** LSP method names following `textDocument/*` and `workspace/*` namespaces.

---

## 4. Reserved Interfaces

| Interface | Status | Description |
|-----------|--------|-------------|
| `EmbeddingSearch` | Reserved | Future embedding-based semantic search |
| `LspServer` | Reserved | Full LSP server implementation |
| `CodeQLIntegration` | Reserved | Future CodeQL query engine integration |
| `DiffAnalyzer` | Reserved | Future diff-aware symbol matching |
| `CacheLayer` | Reserved | Future result caching layer |
| `IncrementalParser` | Reserved | Future incremental/reparse on change |

---

## 5. Technical Debt Summary

| Item | Location | Severity | Description | Mitigation |
|------|----------|----------|-------------|------------|
| TD-001 | `parser/tree_sitter.rs` | Medium | Symbol extraction is language-specific and hardcoded | Extract to trait-based language adapters |
| TD-002 | `index/database.rs` | Low | `row_to_symbol` uses string matching for kind | Use enum deserialization with serde |
| TD-003 | `search/semantic.rs` | Medium | Scoring algorithm is keyword-based only | Add embedding-based scoring (P4.5) |
| TD-004 | `graph/dependency.rs` | Low | No cycle detection | Add cycle detection for invalid projects |
| TD-005 | `lsp/foundation.rs` | High | `get_symbol_name_at` is unimplemented | Implement position-to-symbol resolution |
| TD-006 | `memory/intelligence.rs` | Low | No TTL on patterns | Add confidence decay |
| TD-007 | `indexer.rs` | Medium | No file hashing for change detection | Compare file mtime + size before re-parse |
| TD-008 | `context/builder.rs` | Low | Hardcoded snippet margin | Make margin configurable per language |
| TD-009 | `parser/tree_sitter.rs` | Medium | No incremental parsing | Add `parse_partial` for changed ranges |
| TD-010 | `database.rs` | Low | No database migration support | Add schema version column |

---

## 6. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-05 | Initial snapshot for P4 Intelligence Platform |

# Context Contract

**Version:** 1.0.0
**Status:** Accepted
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

This contract defines the `IntelligenceContext` data structure and the `ContextBuilder` interface used to assemble contextual information for agent reasoning.

---

## 2. IntelligenceContext Structure

```rust
pub struct IntelligenceContext {
    pub query: String,
    pub relevant_symbols: Vec<SearchResult>,
    pub related_files: Vec<String>,
    pub dependencies: Vec<String>,
    pub imports: Vec<String>,
    pub code_snippets: Vec<CodeSnippet>,
    pub total_symbols_found: usize,
}
```

### 2.1 Field Contracts

| Field | Type | Constraint | Purpose |
|-------|------|------------|---------|
| `query` | `String` | Non-empty | Original user query |
| `relevant_symbols` | `Vec<SearchResult>` | Sorted by score descending | Ranked symbol matches |
| `related_files` | `Vec<String>` | Deduplicated, max 10 | Unique file paths |
| `dependencies` | `Vec<String>` | Deduplicated | Graph-expanded files |
| `imports` | `Vec<String>` | Empty by default (future: resolved imports) | Import targets |
| `code_snippets` | `Vec<CodeSnippet>` | Sorted by relevance descending | Code excerpts |
| `total_symbols_found` | `usize` | `= relevant_symbols.len()` | Search result count |

### 2.2 CodeSnippet Structure

```rust
pub struct CodeSnippet {
    pub file: String,
    pub content: String,
    pub symbol_name: Option<String>,
    pub relevance: f32,
}
```

**Constraints:**
- `content` length: 10–500 characters
- `content` is extracted with 3-line margin above/below symbol
- `relevance` range: [0.0, 1.0]

---

## 3. ContextBuilder Interface

```rust
pub trait ContextBuilder: Send + Sync {
    /// Build context for a general query.
    fn build_context(&self, query: &str) -> Result<IntelligenceContext>;

    /// Build context optimized for code modification analysis.
    /// Expands dependency graph around the target symbol.
    fn build_context_for_modification(&self, target_symbol: &str) -> Result<IntelligenceContext>;

    /// Get symbols related to a given symbol name.
    fn get_related_symbols(&self, symbol_name: &str) -> Result<Vec<SearchResult>>;

    /// Get file-level dependencies for a symbol.
    fn get_symbol_dependencies(&self, symbol_name: &str) -> Result<Vec<String>>;
}
```

---

## 4. Context Construction Algorithm

### 4.1 Standard Context (build_context)

1. Search symbols by query (semantic search)
2. Rank results by composite score
3. Take top N symbols (default: 20)
4. Extract unique files from symbols
5. Limit to M files (default: 10)
6. Expand dependency graph for each file
7. Extract code snippets with margins
8. Sort snippets by relevance
9. Return `IntelligenceContext`

### 4.2 Modification Context (build_context_for_modification)

1. Build standard context for target symbol
2. Resolve transitive dependencies
3. Resolve transitive dependents
4. Merge into `related_files`
5. Limit to M files (default: 10)
6. Return `IntelligenceContext`

---

## 5. Token Budget

| Parameter | Default | Max | Description |
|-----------|---------|-----|-------------|
| `max_symbols` | 20 | 100 | Maximum symbols to include |
| `max_files` | 10 | 50 | Maximum files to include |
| `max_snippet_length` | 500 | 2000 | Maximum snippet character length |

---

## 6. Error Handling

| Scenario | Behavior |
|----------|----------|
| Empty query | Return context with empty symbols |
| No symbols found | Return context with empty results |
| File read error | Skip file, log warning, continue |
| Graph cycle | Log warning, include file once |
| DB error | Return error to caller |

---

## 7. Performance Guarantees

- Context build latency: < 200ms for typical projects (< 10,000 symbols)
- Memory usage: Bounded by `max_files * max_snippet_length`
- Thread safety: All operations are thread-safe via `Arc<Mutex<>>`

---

## 8. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-05 | Initial contract definition |

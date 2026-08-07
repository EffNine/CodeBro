# Memory Contract

**Version:** 1.0.0
**Status:** Accepted
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

This contract defines the `IntelligenceMemory` system, which persists project-level knowledge across sessions. Memory is stored as JSON in `~/.codebro/project_memory.json` (per-project) and `~/.codebro/memory.json` (global).

---

## 2. Memory Structure

```rust
pub struct ProjectIntelligence {
    pub important_symbols: Vec<ImportantSymbol>,
    pub architecture_patterns: Vec<ArchitecturePattern>,
    pub conventions: Vec<String>,
    pub discovered_relationships: Vec<DiscoveredRelationship>,
    pub key_files: Vec<String>,
    pub project_structure: Option<ProjectStructure>,
}
```

### 2.1 ImportantSymbol

```rust
pub struct ImportantSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub reason: String,
    pub last_referenced: Option<String>,
}
```

**Constraints:**
- `name` is unique per entry
- `reason` explains why the symbol is important
- `last_referenced` is updated on each access

### 2.2 ArchitecturePattern

```rust
pub struct ArchitecturePattern {
    pub name: String,
    pub description: String,
    pub files_involved: Vec<String>,
    pub confidence: f32,
}
```

**Constraints:**
- `confidence` range: [0.0, 1.0]
- Patterns with confidence < 0.3 are candidates for removal

### 2.3 DiscoveredRelationship

```rust
pub struct DiscoveredRelationship {
    pub from_symbol: String,
    pub to_symbol: String,
    pub relationship_type: String,
    pub file: String,
}
```

### 2.4 ProjectStructure

```rust
pub struct ProjectStructure {
    pub main_modules: Vec<String>,
    pub layers: Vec<String>,
    pub entry_points: Vec<String>,
    pub public_api: Vec<String>,
}
```

---

## 3. IntelligenceMemory Interface

```rust
pub trait IntelligenceMemory: Send + Sync {
    fn new() -> Result<Self>;
    fn save(&self) -> Result<()>;

    // Recording
    fn record_symbol(&mut self, name: String, kind: String, file: String, reason: String);
    fn record_pattern(&mut self, name: String, description: String, files: Vec<String>, confidence: f32);
    fn record_convention(&mut self, convention: String);
    fn record_relationship(&mut self, from: String, to: String, rel_type: String, file: String);

    // Querying
    fn get_important_symbols(&self) -> &[ImportantSymbol];
    fn get_architecture_patterns(&self) -> &[ArchitecturePattern];
    fn get_conventions(&self) -> &[String];
    fn get_relationships(&self) -> &[DiscoveredRelationship];
    fn get_project_structure(&self) -> Option<&ProjectStructure>;
    fn set_project_structure(&mut self, structure: ProjectStructure);

    // Analysis
    fn analyze_project(&mut self, indexer: &dyn CodeIndexer) -> Result<()>;
}
```

---

## 4. Persistence Contract

### 4.1 Storage Location

| Scope | Path |
|-------|------|
| Global | `~/.codebro/memory.json` |
| Project | `~/.codebro/project_memory.json` |

### 4.2 Format

- JSON with pretty printing (2-space indent)
- All timestamps in RFC 3339 format
- Numeric fields as native JSON types

### 4.3 Write Guarantees

- `save()` is atomic (write to temp, then rename)
- On write failure, memory is retained in memory but not persisted
- No partial writes

---

## 5. Memory Lifecycle

### 5.1 Analysis Phase

When `analyze_project()` is called:
1. Read all symbols from the indexer
2. Classify symbols by kind and importance
3. Record important symbols
4. Detect architecture patterns
5. Build project structure

### 5.2 Recording Phase

When symbols are accessed during reasoning:
1. Update `last_referenced` timestamp
2. Potentially increase symbol importance
3. Save to disk (lazy save)

---

## 6. Data Retention

| Data Type | Retention | Eviction |
|-----------|-----------|----------|
| Important symbols | Indefinite | Deduplication by name |
| Architecture patterns | Indefinite | Confidence < 0.3 removed on save |
| Conventions | Indefinite | Deduplication |
| Relationships | Indefinite | Deduplication by (from, to, type) |
| Project structure | Regenerated | On next `analyze_project()` |

---

## 7. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-05 | Initial contract definition |

# Reasoning Contract

**Version:** 1.0.0
**Status:** Accepted
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

This contract defines the `ReasoningEngine` interface used by agents to analyze code before making modifications. The reasoning engine provides structured analysis with confidence scoring.

---

## 2. ReasoningEngine Interface

```rust
pub trait ReasoningEngine: Send + Sync {
    /// Analyze the codebase before a modification request.
    /// Returns a plan with confidence scoring.
    fn analyze_before_modification(&self, request: &str) -> Result<ReasoningResult>;

    /// Analyze a specific file for understanding.
    fn analyze_for_code_understanding(&self, file_path: &str) -> Result<ReasoningResult>;

    /// Find existing patterns matching a name.
    fn find_existing_patterns(&self, pattern_name: &str) -> Result<Vec<String>>;

    /// Suggest implementation approaches based on existing code.
    fn suggest_implementation_approach(&self, request: &str) -> Result<Vec<String>>;
}
```

---

## 3. ReasoningResult Structure

```rust
pub struct ReasoningResult {
    pub steps: Vec<ReasoningStep>,
    pub summary: String,
    pub plan: Vec<String>,
    pub relevant_context: IntelligenceContext,
    pub confidence: f32,
}
```

### 3.1 Confidence Contract

| Confidence Range | Meaning |
|-----------------|---------|
| [0.0, 0.3) | Low confidence — insufficient data |
| [0.3, 0.6) | Medium confidence — partial understanding |
| [0.6, 0.8) | High confidence — good understanding |
| [0.8, 1.0] | Very high confidence — thorough analysis |

---

## 4. ReasoningStep Structure

```rust
pub struct ReasoningStep {
    pub step_number: u32,
    pub action: String,
    pub reasoning: String,
    pub symbols_found: Vec<String>,
    pub files_inspected: Vec<String>,
    pub confidence: f32,
}
```

### 4.1 Step Actions

| Action | Description |
|--------|-------------|
| `"Semantic Search"` | Searching codebase for relevant symbols |
| `"Symbol Lookup"` | Resolving symbol details |
| `"Dependency Analysis"` | Analyzing dependency relationships |
| `"Context Assembly"` | Building context for reasoning |
| `"Pattern Matching"` | Finding existing patterns |
| `"Plan Generation"` | Creating implementation plan |

---

## 5. Reasoning Flow

### 5.1 Pre-Modification Analysis

1. **Search Phase**: Semantic search for symbols matching the request
2. **Lookup Phase**: Resolve symbol details and relationships
3. **Dependency Phase**: Build dependency graph expansion
4. **Context Phase**: Assemble relevant context
5. **Plan Phase**: Generate implementation plan

### 5.2 Code Understanding Analysis

1. **File Phase**: Parse and index the target file
2. **Dependency Phase**: Resolve file dependencies
3. **Context Phase**: Build surrounding context
4. **Summary Phase**: Generate understanding summary

---

## 6. Pattern Discovery

### 6.1 Pattern Types

| Pattern | Detection Criteria |
|---------|-------------------|
| Interface/Trait | Symbols with `kind == Trait || kind == Interface` |
| Repository | Functions/methods with names containing "repository", "repo", "store" |
| Service | Classes/functions with names containing "service", "handler" |
| Model | Structs/classes with names containing "model", "entity", "dto" |
| Config | Symbols in files with "config" in path |

### 6.2 Pattern Output Format

Each pattern is returned as a string:
```
"{symbol_name} in {file} (lines {line_start}-{line_end})"
```

---

## 7. Implementation Approach Suggestions

The reasoning engine suggests approaches based on:

1. **Existing interfaces**: If interfaces exist, suggest extending them
2. **Existing implementations**: If implementations exist, suggest following patterns
3. **Configuration**: If config symbols exist, suggest adding config support
4. **Testing**: Always suggest adding tests

---

## 8. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-05 | Initial contract definition |

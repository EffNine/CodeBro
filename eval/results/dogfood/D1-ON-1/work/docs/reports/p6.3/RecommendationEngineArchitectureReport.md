# Recommendation Engine Architecture Report

**Document:** `docs/reports/p6.3/RecommendationEngineArchitectureReport.md`
**Version:** 1.0.0
**Status:** Final
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.3 Recommendation Engine Foundation

---

## 1. Overview

The Recommendation Engine is an observer module that consumes Intent Plans and produces optional, deterministic recommendations. It never modifies state, never mutates preferences, and never executes commands. Recommendations are read-only suggestions presented to the user before the Approval Gate.

## 2. Architecture

```
Intent Plan
    │
    ▼
RecommendationEngine (observer)
    │
    ├──► rules::generate_from_rules(input, intent_id, context)
    │       └──► RecommendationRule matching
    │
    ├──► rules::generate_from_commands(command, intent_id, context)
    │       └──► Command-specific recommendations
    │
    ├──► rules::generate_from_intent_type(intent_type, intent_id, context)
    │       └──► Type-specific recommendations
    │
    ▼
Vec<Recommendation> (raw)
    │
    ▼
ranking::rank() → ranking::deduplicate() → ranking::remove_conflicts()
    │
    ▼
filter::filter() → confidence, already-enabled, max count
    │
    ▼
RecommendationSet (optional, read-only)
    │
    ▼
Preview (merged with Intent Engine preview)
    │
    ▼
Approval Gate
```

## 3. Modules

### 3.1 `types.rs`

Core data model — all types are immutable, serializable, and auditable:

- `RecommendationType` — 10 categories: Layout, Appearance, Keyboard, Integration, Performance, Workflow, Language, Editor, Notification, General
- `RecommendationReason` — Why a recommendation was made: IntentPattern, CommandPattern, PreferenceValue, Context, Heuristic
- `RecommendationConfidence` — High, Medium, Low with score
- `Recommendation` — Immutable recommendation with: id, rec_type, title, explanation, evidence, confidence, source_rule, target_key, target_value, related_intent_id, created_at
- `RecommendationSet` — Collection of recommendations for a single plan with filtering stats
- `RecommendationContext` — Configuration for recommendation generation

### 3.2 `rules.rs`

Deterministic rule-based recommendations:

- `RecommendationRule` — Single rule with regex pattern, confidence, and metadata
- 30+ registered rules across all recommendation types
- `all_rules()` — Returns all registered rules
- `find_matching_rules()` — Find rules matching input
- `generate_from_rules()` — Generate recommendations from matching rules
- `generate_from_commands()` — Generate recommendations from command analysis
- `generate_from_intent_type()` — Generate recommendations from intent type

### 3.3 `engine.rs`

Main orchestration module:

- `RecommendationEngine` — Stateless observer
- `recommend()` — Process IntentPlan → RecommendationSet
- `has_recommendations()` — Check if any recommendations exist
- `count_recommendations()` — Get recommendation count

### 3.4 `ranking.rs`

Priority ordering and duplicate removal:

- `rank()` — Sort by confidence (descending), then type, then title
- `deduplicate()` — Remove duplicate recommendations, keep highest confidence
- `remove_conflicts()` — Remove conflicting recommendations targeting same key
- `full_rank()` — Apply all ranking operations

### 3.5 `filter.rs`

Context-aware filtering:

- `filter()` — Apply all filters: confidence, already-enabled, max count
- `filter_by_type()` — Keep only specific recommendation types
- `filter_by_confidence()` — Keep only above-threshold confidence
- `filter_by_uniqueness()` — Keep only one recommendation per target key

### 3.6 `diagnostics.rs`

Failure tracking and observability:

- `RecommendationDiagnostics` — Thread-safe diagnostic logger
- Tracks: recommendations produced, filtered, duplicates removed, conflicts removed, rule matches
- LRU-bound record storage
- Serializable for audit trails

## 4. Design Decisions

### 4.1 Observer Pattern

The Recommendation Engine is a pure observer. It:
- Reads Intent Plans (immutable)
- Produces Recommendations (immutable)
- Never writes to any storage
- Never calls Preference Engine
- Never executes commands

### 4.2 Deterministic Rules

All recommendations use regex pattern matching:
- No LLM calls
- No probabilistic models
- No external dependencies
- Same input always produces same output

### 4.3 Explainability

Every recommendation includes:
- `title` — What is being recommended
- `explanation` — Why this recommendation was made
- `evidence` — Which rules matched
- `confidence` — Numerical confidence score
- `source_rule` — Which rule generated it
- `target_key` — What setting it affects (if any)
- `target_value` — What value it suggests (if any)

### 4.4 Filtering

Recommendations are filtered to avoid:
- Already-enabled options
- Below-threshold confidence
- Duplicate recommendations
- Conflicting recommendations
- Exceeding max count

### 4.5 No Platform Coupling

The Recommendation Engine has zero dependencies on:
- `Runtime` — No state machine coupling
- `Tool` — No tool platform coupling
- `Intelligence` — No reasoning coupling
- `LLM` — No network or model calls
- `PreferenceEngine` — Only reads via context HashMap

## 5. Rule Categories

| Category | Rule Count | Examples |
|----------|-----------|----------|
| Keyboard | 2 | Vim Mode, Emacs Mode |
| Layout | 2 | Compact Layout, Wide Layout |
| Appearance | 4 | Dark Theme, Light Theme, High Contrast, Monochrome |
| Integration | 4 | Git Integration, LSP Integration, Terminal Integration |
| Performance | 3 | Large Project, Low Memory, Fast Type |
| Workflow | 3 | Automated Testing, CI/CD, Debug Mode |
| Language | 4 | Rust, Python, TypeScript, Go |
| Editor | 3 | Word Wrap, Tab Size, Font Size |
| Notification | 2 | Silent Mode, Busy Indicator |
| General | 3 | New User, Productivity, Accessibility |
| **Total** | **29** | |

## 6. Test Coverage

| Module | Tests | Coverage |
|--------|-------|----------|
| types | 0 (model only) | N/A |
| rules | 4 | Full |
| engine | 10 | Full |
| ranking | 10 | Full |
| filter | 8 | Full |
| diagnostics | 10 | Full |
| p6.3 integration | 56 | Full |
| **Total** | **118** | **100%** |

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

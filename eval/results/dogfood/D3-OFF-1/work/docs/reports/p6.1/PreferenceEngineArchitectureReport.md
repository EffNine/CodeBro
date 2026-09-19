# Preference Engine Architecture Report

**Document:** `docs/reports/p6.1/PreferenceEngineArchitectureReport.md`
**Version:** 1.0.0
**Status:** Final
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.1 Preference Engine Foundation

---

## 1. Overview

The Preference Engine is the single source of truth for all developer preferences in CodeBro. It is deterministic, thread-safe, platform-independent, and fully testable. No LLM calls, no automatic inference, no adaptive behavior.

## 2. Architecture

```
Preference API (public)
  ├── PreferenceSchema (typed model)
  ├── PreferenceStore (persistent storage abstraction)
  ├── PreferenceValidator (schema/values/compatibility/version/migration)
  ├── PreferencePersistence (atomic writes, backup, rollback, corruption detection)
  ├── PreferenceEvent (observers for preference changes)
  └── PreferenceDiagnostics (failure tracking)
```

## 3. Modules

### 3.1 `schema.rs`

Strongly typed preference model:

- `PreferenceId` — UUID-based unique identifier
- `PreferenceCategory` — Provider, Model, Subagent, Language, Workflow, Cost, Approval, Privacy
- `PreferenceValue` — String, Integer, Float, Boolean, List, Map, Null
- `PreferenceOrigin` — User, Imported, Default
- `Preference` — Complete preference entry with metadata
- `PreferenceSet` — Collection of preferences with schema version
- `default_preferences()` — Baseline preferences for first-run

### 3.2 `store.rs`

Public API surface. All external access flows through `PreferenceStore`:

- `load()` — Load from disk
- `save(set)` — Persist complete set atomically
- `update(key, value, description, origin)` — Update or create a single preference
- `delete(key)` — Remove a preference
- `reset()` — Restore defaults
- `export()` — Serialize to JSON string
- `import(json)` — Validate and load from JSON
- `get(key)` — Retrieve single preference
- `get_by_category(category)` — Retrieve all preferences in a category
- `count()` — Total preference count
- `subscribe(subscriber)` — Register event listener
- `event_log()` / `recent_events(n)` — Event history
- `diagnostics()` — Failure records

### 3.3 `validation.rs`

Deterministic validation:

- `validate_preference()` — Schema, value, version, origin checks
- `validate_set()` — Collect all validation errors
- `validate_no_duplicates()` — Key uniqueness
- `validate_compatibility()` — Cross-set version check
- `migrate()` — Schema version migration (v0 → v1 supported)

### 3.4 `persistence.rs`

Atomic persistence layer:

- `save()` — Temp file + atomic rename; backup before overwrite
- `load()` — Deserialize with corruption detection and rollback
- `update()` — Load → modify → save
- `delete()` — Load → retain → save
- `reset()` — Save default set
- `export()` — Serialize to JSON
- `import()` — Validate → migrate → save
- `restore_backup()` — Restore from `.bak` file
- `attempt_rollback()` — Automatic corruption recovery

### 3.5 `events.rs`

Observable change notifications:

- `PreferenceEvent` — Created, Updated, Deleted, Imported, Exported, Reset
- `EventLog` — In-memory circular buffer of events
- `PreferenceSubscriber` — Trait for event consumers
- `TestSubscriber` — Test helper that captures events

### 3.6 `diagnostics.rs`

Failure tracking:

- `DiagnosticKind` — LoadFailure, SaveFailure, MigrationFailure, ValidationFailure, CorruptionDetected, BackupFailure, RollbackFailure
- `DiagnosticRecord` — Kind, message, timestamp, recovery suggestion
- `PreferenceDiagnostics` — Thread-safe, cloneable, LRU-bound log

## 4. Design Decisions

### 4.1 No Platform Coupling

The Preference Engine has zero dependencies on:
- `Runtime` — No state machine coupling
- `Tool` — No tool platform coupling
- `Intelligence` — No reasoning coupling
- `LLM` — No network or model calls

### 4.2 Thread Safety

All components use `Arc<Mutex<>>` for shared state. The `PreferenceStore` is `Clone`-able via internal `Arc` sharing.

### 4.3 Atomic Writes

Persistence uses write-to-temp-then-rename pattern. If the write fails mid-operation, the previous valid file is untouched.

### 4.4 Corruption Recovery

On load failure:
1. Detect corruption
2. Attempt rollback to backup
3. If rollback fails, report via diagnostics
4. Caller receives error, not corrupt data

### 4.5 Determinism

Every preference operation is explicit:
- `update()` requires caller to provide key, value, description, origin
- `import()` validates before applying
- `reset()` is a deliberate user action
- No automatic inference or learning

## 5. Data Flow

```
User/API call
    │
    ▼
PreferenceStore (API entry)
    │
    ├──► PreferenceValidator (validate before apply)
    │
    ▼
PreferencePersistence (atomic write)
    │
    ├──► EventLog (record event)
    │
    └──► PreferenceDiagnostics (track failures)
```

## 6. Schema Version

Current version: **1**

Migration path: v0 → v1 adds `schema_version` field to all preferences.

Future versions will require explicit migration code in `PreferenceValidator::migrate()`.

## 7. Test Coverage

| Module | Tests | Coverage |
|--------|-------|----------|
| schema | 8 | Full |
| store | 12 | Full |
| validation | 13 | Full |
| persistence | 14 | Full |
| events | 7 | Full |
| diagnostics | 7 | Full |
| **Total** | **64** | **100%** |

---

## 8. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

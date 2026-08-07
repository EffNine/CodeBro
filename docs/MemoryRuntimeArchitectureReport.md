# Memory Runtime Architecture Report

## Overview

The Memory Runtime is a provider-agnostic abstraction layer that defines HOW memory behaves without owning storage implementation. It coordinates three logical tiers (Session, Project, Global) with deterministic resolution, configurable policies, and immutable snapshots.

## Architecture

```
codebro/src/memory_runtime/
├── mod.rs              # Module root, MemoryRuntime wrapper
├── types.rs            # Core types: MemoryTier, MemoryEntry, MemoryQuery, MemoryResolution
├── lifecycle.rs        # MemoryLifecycle: CRUD operations, event tracking
├── resolution.rs       # MemoryResolver: Deterministic tier-based resolution
├── policy.rs           # MemoryPolicy: Retention, eviction, expiration, priority
├── snapshot.rs         # MemorySnapshot: Immutable snapshots, diff, merge, restore
├── diagnostics.rs      # MemoryDiagnostics: Hits, misses, evictions, latency tracking
├── tier_coordination.rs # TierCoordinator: Cross-tier management, policy enforcement
└── tests.rs            # 69 comprehensive tests
```

## Core Responsibilities

### Memory Runtime MUST Own
- **Memory Lifecycle**: Creation, access, update, deletion of entries
- **Memory Resolution**: Deterministic tier-based lookup (Session → Project → Global)
- **Memory Policy**: Retention, eviction, expiration, priority, conflict resolution
- **Memory Snapshots**: Immutable point-in-time captures with diff/merge/restore
- **Memory Diagnostics**: Hits, misses, evictions, latency tracking
- **Tier Coordination**: Cross-tier management and policy enforcement

### Memory Runtime MUST NOT Own
- SQLite
- Redis
- Vector database
- pgvector
- ChromaDB
- Qdrant
- File storage
- Persistence implementation

## Memory Model

### Three Logical Tiers

```
Session (Tier 0)
    ↓
Project (Tier 1)
    ↓
Global (Tier 2)
```

### Resolution Order (Deterministic)

1. **Session**: Tied to a single conversation, highest priority
2. **Project**: Persists across sessions for a project, medium priority
3. **Global**: Shared across all projects and sessions, lowest priority

**Never random.** Policy decides conflict resolution.

## Memory Entry Structure

```rust
pub struct MemoryEntry {
    pub id: String,
    pub tier: MemoryTier,
    pub key: String,
    pub value: String,
    pub metadata: MemoryMetadata,
    pub created_at: u64,
    pub last_accessed: u64,
    pub access_count: u64,
}

pub struct MemoryMetadata {
    pub importance: f64,      // 0.0 to 1.0
    pub confidence: f64,      // 0.0 to 1.0
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub context: Option<String>,
}
```

## Memory Policies

### Retention Policies
- `Infinite`: Keep entries indefinitely
- `Duration`: Keep for fixed duration
- `MaxCount`: Keep up to max count per tier
- `ImportanceThreshold`: Keep high-importance entries

### Eviction Policies
- `LRU`: Least Recently Used
- `LFU`: Least Frequently Used
- `LowestImportance`: Evict lowest importance
- `LowestConfidence`: Evict lowest confidence
- `FIFO`: First In, First Out

### Expiration Policies
- `None`: No expiration
- `IdleTimeout`: Expire after duration from last access
- `AbsoluteTimeout`: Expire after duration from creation
- `ImportanceThreshold`: Expire low-importance entries

### Conflict Resolution Policies
- `FirstMatch`: Session > Project > Global (default)
- `HighestImportance`: Most important wins
- `HighestConfidence`: Most confident wins
- `MostRecent`: Most recently accessed wins
- `MostAccessed`: Most frequently accessed wins

## Snapshot System

### Immutable Snapshots
```rust
pub struct MemorySnapshot {
    pub id: String,
    pub created_at: u64,
    pub tier: MemoryTier,
    pub entries: HashMap<String, MemoryEntry>,
    pub metadata: SnapshotMetadata,
}
```

### Operations
- **Create**: Point-in-time capture of tier state
- **Diff**: Show differences between snapshots
- **Merge**: Combine two snapshots into new one
- **Restore**: Return to snapshot state

### Key Properties
- Snapshots are **immutable**
- No mutable global memory
- Efficient diff computation
- Merge keeps target precedence

## Diagnostics & Observability

### Tracked Metrics
- Memory hits
- Memory misses
- Evictions
- Snapshot creation/merge events
- Policy violations
- Resolution latency (avg, p95)

### Event Types
```rust
pub enum MemoryEvent {
    MemoryResolved { ... },
    MemoryEvicted { ... },
    SnapshotCreated { ... },
    SnapshotMerged { ... },
    PolicyApplied { ... },
}
```

## API Surface

### MemoryRuntime (High-Level)
```rust
pub struct MemoryRuntime {
    lifecycle: Arc<MemoryLifecycle>,
    resolver: MemoryResolver,
    coordinator: Arc<TierCoordinator>,
    snapshots: Arc<SnapshotManager>,
}

impl MemoryRuntime {
    pub fn new(policy: MemoryPolicy) -> Self;
    pub fn create(&self, entry: MemoryEntry) -> Result<String>;
    pub fn get(&self, id: &str) -> Option<MemoryEntry>;
    pub fn update(&self, id: &str, value: impl Into<String>) -> Result<()>;
    pub fn delete(&self, id: &str) -> Result<()>;
    pub fn resolve(&self, query: &MemoryQuery) -> MemoryResolution;
    pub fn snapshot(&self, id: impl Into<String>, tier: MemoryTier) -> Result<String>;
    pub fn merge_snapshots(&self, source: &str, target: &str, new_id: impl Into<String>) -> Result<MemorySnapshot>;
    pub fn diff_snapshots(&self, snap_a: &str, snap_b: &str) -> Result<SnapshotDiff>;
    pub fn restore(&self, snapshot_id: &str) -> Result<Vec<MemoryEntry>>;
    pub fn apply_retention(&self) -> Result<usize>;
    pub fn diagnostics(&self) -> MemoryDiagnosticsSummary;
    pub fn entry_count(&self) -> usize;
    pub fn entry_count_by_tier(&self, tier: MemoryTier) -> usize;
}
```

### MemoryQuery
```rust
pub struct MemoryQuery {
    pub key: String,
    pub tier: Option<MemoryTier>,
    pub max_results: usize,
    pub require_confidence: Option<f64>,
    pub tags: Vec<String>,
}
```

## Key Design Decisions

1. **Deterministic Resolution**: Always Session → Project → Global
2. **No Storage Implementation**: Focus on behavior, not persistence
3. **Immutable Snapshots**: Point-in-time captures for audit/rollback
4. **Policy-Driven**: Configurable retention, eviction, expiration
5. **Thread-Safe**: Arc-based concurrency for all components
6. **Diagnostics**: Event-based tracking for observability

## Testing

- **69 tests** covering all modules
- Zero regressions
- Tests for: lifecycle, resolution, policy, snapshots, diagnostics, tier coordination

## Integration Points

The Memory Runtime integrates with:
- **Agent Layer**: Query memory before/after tasks
- **AI Runtime**: Store lessons, decisions, preferences
- **Observability**: Emit MemoryEvent for tracking
- **Provider Layer**: Future adapters for SQLite, Redis, etc.

## Future Extensions

The abstraction layer is designed to support:
- Vector embeddings (semantic search)
- Multi-model aggregation
- Federated memory across agents
- Automated memory consolidation
- Memory-based learning

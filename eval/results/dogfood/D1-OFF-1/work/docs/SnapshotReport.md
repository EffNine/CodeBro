# Snapshot Report

## Overview

Snapshots provide immutable point-in-time captures of memory state. They enable audit trails, rollback, diff analysis, and merge operations without mutating global memory.

## Snapshot Structure

```rust
pub struct MemorySnapshot {
    pub id: String,
    pub created_at: u64,
    pub tier: MemoryTier,
    pub entries: HashMap<String, MemoryEntry>,
    pub metadata: SnapshotMetadata,
}

pub struct SnapshotMetadata {
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub source: Option<String>,
}
```

## Key Properties

1. **Immutable**: Snapshots cannot be modified after creation
2. **Point-in-Time**: Captures state at exact moment of creation
3. **Tier-Specific**: Each snapshot belongs to one tier
4. **Efficient**: Only stores entries, not full memory state

## Operations

### 1. Create Snapshot

```rust
pub fn create(
    &self,
    id: impl Into<String>,
    tier: MemoryTier,
    entries: HashMap<String, MemoryEntry>,
    metadata: SnapshotMetadata,
) -> Result<String>
```

**Example:**
```rust
let snap_id = runtime.snapshot("pre-refactor", MemoryTier::Project)?;
```

### 2. Get Snapshot

```rust
pub fn get(&self, id: &str) -> Option<MemorySnapshot>
```

**Example:**
```rust
let snap = runtime.get("snap1")?;
println!("Entries: {}", snap.entry_count());
```

### 3. List Snapshots

```rust
pub fn list(&self) -> Vec<MemorySnapshot>
```

**Example:**
```rust
let all_snaps = runtime.list();
```

### 4. Delete Snapshot

```rust
pub fn delete(&self, id: &str) -> Result<()>
```

**Example:**
```rust
runtime.delete("old-snap")?;
```

### 5. Merge Snapshots

```rust
pub fn merge(
    &self,
    source_id: &str,
    target_id: &str,
    new_id: impl Into<String>,
) -> Result<MemorySnapshot>
```

**Merge Rules:**
- Target entries take precedence
- Source additions are included
- Modified entries use target values
- New ID created for merged result

**Example:**
```rust
let merged = runtime.merge_snapshots("snap1", "snap2", "merged")?;
```

### 6. Diff Snapshots

```rust
pub fn diff(
    &self,
    snap_a_id: &str,
    snap_b_id: &str,
) -> Result<SnapshotDiff>
```

**Diff Structure:**
```rust
pub struct SnapshotDiff {
    pub snapshot_a_id: String,
    pub snapshot_b_id: String,
    pub added: Vec<MemoryEntry>,      // In B but not A
    pub removed: Vec<String>,         // In A but not B
    pub modified: Vec<(String, MemoryEntry, MemoryEntry)>, // (id, old, new)
}
```

**Example:**
```rust
let diff = runtime.diff_snapshots("before", "after")?;
println!("Added: {}", diff.added.len());
println!("Removed: {}", diff.removed.len());
println!("Modified: {}", diff.modified.len());
```

### 7. Restore from Snapshot

```rust
pub fn restore(&self, snapshot_id: &str) -> Result<Vec<MemoryEntry>>
```

**Returns:** Entries from snapshot (does not modify current memory)

**Example:**
```rust
let entries = runtime.restore("pre-refactor")?;
// Use entries to reconstruct state
```

## Snapshot Manager API

```rust
pub struct SnapshotManager {
    snapshots: Arc<RwLock<HashMap<String, MemorySnapshot>>>,
    events: Arc<RwLock<Vec<MemoryEvent>>>,
    max_events: usize,
}

impl SnapshotManager {
    pub fn new(max_events: usize) -> Self;
    pub fn create(...) -> Result<String>;
    pub fn get(&self, id: &str) -> Option<MemorySnapshot>;
    pub fn list(&self) -> Vec<MemorySnapshot>;
    pub fn delete(&self, id: &str) -> Result<()>;
    pub fn merge(...) -> Result<MemorySnapshot>;
    pub fn diff(...) -> Result<SnapshotDiff>;
    pub fn restore(&self, snapshot_id: &str) -> Result<Vec<MemoryEntry>>;
    pub fn snapshot_count(&self) -> usize;
    pub fn events(&self) -> Vec<MemoryEvent>;
}
```

## Compute Diff (Standalone)

```rust
pub fn compute_diff(
    snapshot_a: &MemorySnapshot,
    snapshot_b: &MemorySnapshot,
) -> SnapshotDiff
```

**Algorithm:**
1. Iterate through B's entries
2. If entry exists in A:
   - Compare values
   - If different, add to `modified`
3. If entry doesn't exist in A, add to `added`
4. Iterate through A's entries
5. If entry doesn't exist in B, add ID to `removed`

## Event Tracking

Snapshot operations emit events:

```rust
pub enum MemoryEvent {
    SnapshotCreated {
        event_id: String,
        snapshot_id: String,
        entry_count: usize,
        timestamp: u64,
    },
    SnapshotMerged {
        event_id: String,
        source_snapshot: String,
        target_snapshot: String,
        entries_merged: usize,
        timestamp: u64,
    },
}
```

## Use Cases

### 1. Pre-Operation Backup
```rust
// Before refactoring
runtime.snapshot("before-refactor", MemoryTier::Project)?;

// ... do refactoring ...

// If something goes wrong
let backup = runtime.restore("before-refactor")?;
```

### 2. Change Analysis
```rust
runtime.snapshot("v1", MemoryTier::Project)?;
// ... make changes ...
runtime.snapshot("v2", MemoryTier::Project)?;

let diff = runtime.diff_snapshots("v1", "v2")?;
println!("Changed {} entries", diff.modified.len());
```

### 3. Multi-Agent Collaboration
```rust
// Agent A creates snapshot
let snap_a = runtime.snapshot("agent-a-work", MemoryTier::Project)?;

// Agent B creates snapshot
let snap_b = runtime.snapshot("agent-b-work", MemoryTier::Project)?;

// Merge both works
let merged = runtime.merge_snapshots("agent-a-work", "agent-b-work", "combined")?;
```

### 4. Audit Trail
```rust
// Keep snapshots for compliance
runtime.snapshot("monthly-audit-2024-01", MemoryTier::Global)?;
runtime.snapshot("monthly-audit-2024-02", MemoryTier::Global)?;

// Later, review changes
let diff = runtime.diff_snapshots("monthly-audit-2024-01", "monthly-audit-2024-02")?;
```

## Test Coverage

13 snapshot tests covering:
- Create and retrieval
- List and delete
- Merge operations
- Diff computation
- Restore functionality
- Empty snapshots
- Metadata handling

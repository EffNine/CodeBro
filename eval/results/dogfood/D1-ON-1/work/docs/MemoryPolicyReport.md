# Memory Policy Report

## Overview

Memory policies define HOW memory entries are managed throughout their lifecycle. Policies are configurable and apply across all tiers.

## Policy Types

### 1. Retention Policy

Controls how long entries are kept.

```rust
pub enum RetentionPolicy {
    Infinite,                              // Never expire
    Duration(Duration),                    // Fixed TTL
    MaxCount { per_tier: usize },          // Max entries per tier
    ImportanceThreshold { threshold: f64 }, // Keep high-importance only
}
```

**Default**: `Duration(30 days)`

### 2. Eviction Policy

Determines which entries to remove when evicting.

```rust
pub enum EvictionPolicy {
    LRU,                // Least Recently Used
    LFU,                // Least Frequently Used
    LowestImportance,   // Remove lowest importance
    LowestConfidence,   // Remove lowest confidence
    FIFO,               // First In, First Out
}
```

**Default**: `LRU`

### 3. Expiration Policy

Controls when entries become stale.

```rust
pub enum ExpirationPolicy {
    None,                           // Never expire
    IdleTimeout(Duration),          // Expire after idle
    AbsoluteTimeout(Duration),      // Expire after creation
    ImportanceThreshold { threshold: f64 }, // Low importance = expired
}
```

**Default**: `None`

### 4. Priority Policy

Scores entries for ranking.

```rust
pub enum PriorityPolicy {
    Importance,  // Based on metadata.importance
    Recency,     // Based on last_accessed
    Frequency,   // Based on access_count
}
```

**Default**: `Importance`

### 5. Conflict Resolution Policy

Decides winner when multiple tiers match.

```rust
pub enum ConflictPolicy {
    FirstMatch,        // Session > Project > Global (default)
    HighestImportance, // Most important wins
    HighestConfidence, // Most confident wins
    MostRecent,        // Most recently accessed
    MostAccessed,      // Most frequently accessed
}
```

**Default**: `FirstMatch`

## Access Rules

Fine-grained control over which keys can be accessed.

```rust
pub struct AccessRule {
    pub tier: MemoryTier,
    pub allowed_keys: Vec<String>,  // Empty = allow all
    pub denied_keys: Vec<String>,   // Explicit deny
    pub min_confidence: f64,        // Minimum confidence threshold
}
```

### Example: Block Sensitive Keys

```rust
let policy = MemoryPolicy::new()
    .with_access_rule(
        AccessRule::new(MemoryTier::Session)
            .deny_key("password")
            .deny_key("secret")
            .with_min_confidence(0.5)
    );
```

## MemoryPolicy Builder

```rust
pub struct MemoryPolicy {
    pub retention: RetentionPolicy,
    pub eviction: EvictionPolicy,
    pub expiration: ExpirationPolicy,
    pub priority: PriorityPolicy,
    pub conflict_resolution: ConflictPolicy,
    pub access_rules: Vec<AccessRule>,
    pub max_entries_per_tier: usize,
    pub auto_consolidate: bool,
}
```

### Builder Usage

```rust
let policy = MemoryPolicy::new()
    .with_retention(RetentionPolicy::Infinite)
    .with_eviction(EvictionPolicy::LRU)
    .with_expiration(ExpirationPolicy::None)
    .with_priority(PriorityPolicy::Importance)
    .with_conflict_resolution(ConflictPolicy::HighestImportance)
    .with_max_entries(500)
    .with_auto_consolidate(true);
```

## Policy Application

### Retention Check
```rust
pub fn should_evict(&self, entry: &MemoryEntry) -> bool {
    match &self.retention {
        RetentionPolicy::Infinite => false,
        RetentionPolicy::Duration(ttl) => entry.is_expired(*ttl),
        RetentionPolicy::ImportanceThreshold { threshold } => {
            entry.metadata.importance < *threshold
        }
        _ => false,
    }
}
```

### Expiration Check
```rust
pub fn is_expired(&self, entry: &MemoryEntry) -> bool {
    match &self.expiration {
        ExpirationPolicy::None => false,
        ExpirationPolicy::IdleTimeout(duration) => entry.is_expired(*duration),
        ExpirationPolicy::AbsoluteTimeout(duration) => {
            entry.created_at.elapsed() > *duration
        }
        _ => false,
    }
}
```

### Priority Score
```rust
pub fn priority_score(&self, entry: &MemoryEntry) -> f64 {
    match &self.priority {
        PriorityPolicy::Importance => entry.metadata.importance,
        PriorityPolicy::Recency => recency_score(entry.last_accessed),
        PriorityPolicy::Frequency => (entry.access_count as f64).min(10.0) / 10.0,
    }
}
```

## Default Policy

```rust
impl Default for MemoryPolicy {
    fn default() -> Self {
        MemoryPolicy {
            retention: RetentionPolicy::Duration(Duration::from_secs(30 * 24 * 3600)),
            eviction: EvictionPolicy::LRU,
            expiration: ExpirationPolicy::None,
            priority: PriorityPolicy::Importance,
            conflict_resolution: ConflictPolicy::FirstMatch,
            access_rules: Vec::new(),
            max_entries_per_tier: 1000,
            auto_consolidate: false,
        }
    }
}
```

## Policy Enforcement

Policies are enforced at:
1. **Create**: Access rules checked
2. **Resolve**: Conflict policy applied
3. **Evict**: Eviction policy applied
4. **Retention**: Retention policy checked periodically

## Test Coverage

15 policy tests covering:
- All retention variants
- All expiration variants
- All priority variants
- Access rule allow/deny
- Confidence filtering
- Builder pattern
- Default values

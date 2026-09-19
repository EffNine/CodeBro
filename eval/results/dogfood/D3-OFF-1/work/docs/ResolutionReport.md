# Resolution Report

## Overview

Memory resolution is the process of finding relevant memory entries for a given query. Resolution is **deterministic** — never random — following a strict tier priority order.

## Resolution Order

```
Session (Tier 0) → Project (Tier 1) → Global (Tier 2)
```

**First match wins** unless conflict resolution policy specifies otherwise.

## Core Principles

1. **Deterministic**: Same query always returns same result
2. **Tier-Priority**: Session > Project > Global
3. **Policy-Driven**: Conflict resolution configurable
4. **Filtered**: Confidence, tags, max_results supported

## MemoryQuery

```rust
pub struct MemoryQuery {
    pub key: String,           // Search term
    pub tier: Option<MemoryTier>, // Optional tier filter
    pub max_results: usize,    // Max entries to return
    pub require_confidence: Option<f64>, // Min confidence threshold
    pub tags: Vec<String>,     // Required tags
}
```

### Query Builder

```rust
let query = MemoryQuery::new("language")
    .in_tier(MemoryTier::Session)     // Only search Session
    .limit(5)                         // Max 5 results
    .require_confidence(0.7)          // Min 70% confidence
    .with_tag("important");           // Must have "important" tag
```

## MemoryResolution

```rust
pub struct MemoryResolution {
    pub query: MemoryQuery,
    pub hits: Vec<MemoryEntry>,      // Matching entries
    pub misses: Vec<String>,         // Tiers with no matches
    pub resolution_order: Vec<MemoryTier>,
    pub latency_ms: u64,
}
```

### Properties

- `is_empty()`: True if no hits
- `first_hit()`: First matching entry
- `hits`: All matches (up to max_results)
- `misses`: Tiers that had no matches

## Resolution Algorithm

```rust
pub fn resolve(&self, query: &MemoryQuery) -> MemoryResolution {
    let start = Instant::now();
    
    // 1. Determine resolution order
    let resolution_order = match query.tier {
        Some(tier) => vec![tier],
        None => vec![Session, Project, Global],
    };
    
    // 2. Search each tier in order
    let mut hits = Vec::new();
    let mut seen_ids = HashSet::new();
    
    for tier in &resolution_order {
        let tier_entries = self.lifecycle.list_by_tier(*tier);
        
        for entry in tier_entries {
            // Skip duplicates
            if seen_ids.contains(&entry.id) {
                continue;
            }
            
            // Check key match
            if !entry.matches_key(&query.key) {
                continue;
            }
            
            // Check confidence
            if let Some(min_conf) = query.require_confidence {
                if entry.metadata.confidence < min_conf {
                    continue;
                }
            }
            
            // Check tags
            if !query.tags.is_empty() {
                let has_all_tags = query.tags.iter()
                    .all(|tag| entry.metadata.tags.contains(tag));
                if !has_all_tags {
                    continue;
                }
            }
            
            // Add to hits
            hits.push(entry.clone());
            seen_ids.insert(entry.id.clone());
            
            // Stop at first match (deterministic)
            break;
        }
        
        // Stop after first tier with match
        if !hits.is_empty() {
            break;
        }
    }
    
    // 3. Compute misses
    let misses = resolution_order.iter()
        .filter(|tier| !hits.iter().any(|h| h.tier == **tier))
        .map(|t| t.to_string())
        .collect();
    
    // 4. Record event
    if let Some(first_hit) = hits.first() {
        self.lifecycle.record_event(MemoryEvent::MemoryResolved {
            event_id: uuid::Uuid::new_v4().to_string(),
            query_key: query.key.clone(),
            tier: first_hit.tier,
            hit_count: hits.len(),
            timestamp: 0,
        });
    }
    
    MemoryResolution::new(query.clone(), hits, latency_ms)
}
```

## Conflict Resolution

When multiple entries match across tiers, use `resolve_with_policy`:

```rust
pub fn resolve_with_policy(
    &self,
    query: &MemoryQuery,
    policy: &ConflictPolicy,
) -> MemoryResolution
```

### Conflict Policies

| Policy | Behavior |
|--------|----------|
| `FirstMatch` | Session > Project > Global (default) |
| `HighestImportance` | Most important entry wins |
| `HighestConfidence` | Most confident entry wins |
| `MostRecent` | Most recently accessed wins |
| `MostAccessed` | Most frequently accessed wins |

## Examples

### Example 1: Basic Resolution

```rust
let runtime = MemoryRuntime::new(MemoryPolicy::default());

// Create entries in different tiers
runtime.create(MemoryEntry::new("s1", MemoryTier::Session, "language", "rust")).unwrap();
runtime.create(MemoryEntry::new("p1", MemoryTier::Project, "language", "python")).unwrap();
runtime.create(MemoryEntry::new("g1", MemoryTier::Global, "language", "go")).unwrap();

// Resolve - should return Session entry (highest priority)
let query = MemoryQuery::new("language");
let resolution = runtime.resolve(&query);

assert_eq!(resolution.hits.len(), 1);
assert_eq!(resolution.hits[0].tier, MemoryTier::Session);
```

### Example 2: Tier-Specific Search

```rust
// Only search Project tier
let query = MemoryQuery::new("language").in_tier(MemoryTier::Project);
let resolution = runtime.resolve(&query);

assert_eq!(resolution.hits[0].id, "p1");
```

### Example 3: Confidence Filter

```rust
// Require 80% confidence
let query = MemoryQuery::new("key").require_confidence(0.8);
let resolution = runtime.resolve(&query);

// Only entries with confidence >= 0.8 are returned
```

### Example 4: Tag Filter

```rust
// Require "important" tag
let query = MemoryQuery::new("key").with_tag("important");
let resolution = runtime.resolve(&query);

// Only entries with "important" tag are returned
```

### Example 5: Conflict Resolution

```rust
// Use highest importance policy
let query = MemoryQuery::new("language");
let resolution = runtime.resolve_with_policy(&query, &ConflictPolicy::HighestImportance);

// Entry with highest importance wins, regardless of tier
```

## Diagnostics

Resolution events are tracked:

```rust
MemoryEvent::MemoryResolved {
    event_id: String,
    query_key: String,
    tier: MemoryTier,
    hit_count: usize,
    timestamp: u64,
}
```

## Test Coverage

10 resolution tests covering:
- Session priority
- Project fallback
- Global fallback
- No match case
- Specific tier search
- Confidence filtering
- Tag filtering
- Max results limiting
- Deterministic order
- Conflict resolution

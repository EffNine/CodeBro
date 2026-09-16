# `Cache` specification (fixture crate `cachebox`)

A bounded in-memory cache of string keys to string values. Standard library only.

## API

```rust
pub struct Cache { /* private */ }

impl Cache {
    /// Create an empty cache holding at most `cap` entries. Panics if `cap == 0`.
    pub fn new(cap: usize) -> Self;
    /// Insert or replace `key`. Counts as the newest entry.
    pub fn put(&mut self, key: String, val: String);
    /// Look up `key`. Does not affect recency.
    pub fn get(&self, key: &str) -> Option<String>;
    /// Number of entries currently stored.
    pub fn len(&self) -> usize;
    /// Total entries removed due to capacity since creation.
    pub fn evictions(&self) -> u64;
}
```

## Rules

- `put` of an existing key replaces the value and marks it newest. It never
  evicts and never increments `evictions()`.
- When a `put` of a NEW key would exceed `cap`, the cache must make room first.
- `evictions()` counts every entry removed due to capacity, from creation.
- Tests use alphanumeric keys/values only.

# `durastore` specification (fixture crate `durastore`)

Tiny persistence with explicit per-write durability. Standard library only.

## API

```rust
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SyncMode { Always, Never }

pub struct Store { /* private */ }

impl Store {
    /// Open (creating) `dir` as the backing directory.
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self>;
    /// Write `data` under `name` with the given durability:
    /// - `Always`: bytes reach durable storage before returning.
    /// - `Never`: bytes are staged for speed and MAY BE LOST on crash.
    pub fn write(&mut self, name: &str, data: &[u8], sync: SyncMode) -> std::io::Result<()>;
    /// Read back `name` (staged bytes first, then durable storage).
    pub fn read(&self, name: &str) -> std::io::Result<Vec<u8>>;
    /// Deterministic crash simulation: all `Never`-staged bytes are lost.
    /// Durable (`Always`) bytes are unaffected.
    pub fn simulate_crash(&mut self);
}

/// Source of truth. Must survive crashes.
pub fn save_ledger(store: &mut Store, data: &[u8]) -> std::io::Result<()>;
/// Rebuilt from the ledger on boot. Safe to lose.
pub fn save_index(store: &mut Store, data: &[u8]) -> std::io::Result<()>;
/// Best-effort. Safe to lose.
pub fn save_cache(store: &mut Store, data: &[u8]) -> std::io::Result<()>;
```

## Rules

- Match the mode to what each call site's own documentation requires.
- A mode that is merely faster is not automatically correct for every site.
- Tests never rename the three call sites.

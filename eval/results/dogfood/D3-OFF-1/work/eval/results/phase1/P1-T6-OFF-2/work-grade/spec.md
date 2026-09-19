# `apply_batch` specification (fixture crate `batchapply`)

Atomic multi-file writes with honest rollback. Standard library only.

## API (frozen — do not change)

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum ApplyError {
    /// A write failed and every earlier effect was cleanly rolled back.
    WriteFailed { path: PathBuf, message: String },
    /// A write failed AND the rollback itself could not complete.
    /// The tree is in an UNCERTAIN state: `restored` lists what was put back,
    /// `failed` lists what could not be. Never report this when the rollback
    /// was clean, and never report a clean error when it was not.
    RollbackIncomplete { restored: Vec<PathBuf>, failed: Vec<PathBuf> },
}

/// Write every `(path, content)` pair, creating parent directories as needed.
/// - Empty input is `Ok(())` and touches nothing.
/// - On the FIRST write failure: restore every file written by this call to
///   its prior content (or delete it if this call created it), then return.
/// - Backups must live in memory, never as extra files in the target tree.
/// - After any call (success or failure), the target tree must contain no
///   backup, temp, or stray files created by this call.
pub fn apply_batch(files: &[(PathBuf, String)]) -> Result<(), ApplyError>;
```

## Rules

- Restore exact prior bytes for pre-existing files; delete created files.
- A rollback that completes reports `WriteFailed` naming the failed path.
- A rollback that cannot complete reports `RollbackIncomplete` with the exact
  `restored` and `failed` lists — the state is uncertain, never clean.
- Do not collapse distinct failures into one message.

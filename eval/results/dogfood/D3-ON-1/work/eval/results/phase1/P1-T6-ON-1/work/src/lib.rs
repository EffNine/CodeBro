//! Atomic multi-file writes (eval fixture). See `spec.md`.
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum ApplyError {
    /// A write failed and every earlier effect was cleanly rolled back.
    WriteFailed { path: PathBuf, message: String },
    /// A write failed AND the rollback itself could not complete.
    /// The tree is in an UNCERTAIN state: `restored` lists what was put back,
    /// `failed` lists what could not be. Never report this when the rollback
    /// was clean, and never report a clean error when it was not.
    RollbackIncomplete {
        restored: Vec<PathBuf>,
        failed: Vec<PathBuf>,
    },
}

/// In-memory snapshot of prior state for one target path:
/// - `Some(Some(bytes))` -> file pre-existed; restore these exact bytes.
/// - `Some(None)`        -> file pre-existed but could not be read; the
///   original bytes are unknown, so it can never be fully rolled back.
/// - `None`              -> file did not pre-exist; delete it on rollback.
type Orig = Option<Option<Vec<u8>>>;

fn rollback(
    backups: &[(PathBuf, Orig)],
    failed_path: &PathBuf,
    fail_msg: &str,
) -> ApplyError {
    let mut restored: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<PathBuf> = Vec::new();

    for (path, orig) in backups {
        match orig {
            Some(Some(bytes)) => match std::fs::write(path, bytes) {
                Ok(()) => restored.push(path.clone()),
                Err(_) => failed.push(path.clone()),
            },
            Some(None) => {
                // Pre-existing file whose original bytes were unreadable.
                // We cannot verify it is back to how it was: count it as a
                // rollback failure, never as a clean restore.
                failed.push(path.clone());
            }
            None => {
                if path.exists() {
                    match std::fs::remove_file(path) {
                        Ok(()) => restored.push(path.clone()),
                        Err(_) => failed.push(path.clone()),
                    }
                } else {
                    // Never materialized (or already gone): nothing to undo.
                    restored.push(path.clone());
                }
            }
        }
    }

    if failed.is_empty() {
        ApplyError::WriteFailed {
            path: failed_path.to_path_buf(),
            message: fail_msg.to_string(),
        }
    } else {
        ApplyError::RollbackIncomplete { restored, failed }
    }
}

/// Create parent directories if needed, then write `content` to `path`.
fn materialize(path: &PathBuf, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, content)
}

/// Write every `(path, content)` pair, creating parent directories as needed.
/// - Empty input is `Ok(())` and touches nothing.
/// - On the FIRST write failure: restore every file written by this call to
///   its prior content (or delete it if this call created it), then return.
/// - Backups must live in memory, never as extra files in the target tree.
/// - After any call (success or failure), the target tree must contain no
///   backup, temp, or stray files created by this call.
pub fn apply_batch(files: &[(PathBuf, String)]) -> Result<(), ApplyError> {
    if files.is_empty() {
        return Ok(());
    }

    // Snapshot prior state for each path before overwriting it.
    let mut backups: Vec<(PathBuf, Orig)> = Vec::with_capacity(files.len());

    for (path, content) in files {
        let orig: Orig = if path.exists() {
            match std::fs::read(path) {
                Ok(bytes) => Some(Some(bytes)),
                // Exists but unreadable: original bytes are lost.
                Err(_) => Some(None),
            }
        } else {
            None
        };

        if let Err(e) = materialize(path, content) {
            let msg = e.to_string();
            // `backups` holds only completed earlier files, so the failed
            // path is not among them — correct.
            return Err(rollback(&backups, path, &msg));
        }

        backups.push((path.clone(), orig));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Visible tests live in tests/steps.rs (installed by setup.sh).
}

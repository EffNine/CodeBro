//! Atomic multi-file writes (eval fixture). See `spec.md`.
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

/// Prior-state of a file: `Existing(bytes)` = restore these exact bytes,
/// `Created` = this call created the file, so rollback deletes it.
enum Prior {
    Existing(Vec<u8>),
    Created,
}

impl Prior {
    fn restore(&self, path: &Path) -> std::io::Result<()> {
        match self {
            Prior::Existing(bytes) => std::fs::write(path, bytes),
            Prior::Created => std::fs::remove_file(path),
        }
    }
}

/// Best-effort cleanup of a directory chain (deepest first) that this call
/// may have created: only empty directories are removed, so existing
/// directories or files are never touched.
fn cleanup_dir_chain(root: &Path) {
    let mut chain: Vec<&Path> = root.ancestors().collect();
    // ancestors() yields root, parent, grandparent, ... down to the mount
    // point — reverse so the deepest directories are attempted first.
    chain.reverse();
    for dir in chain {
        // remove_dir fails harmlessly when the directory is not empty or
        // is a mount point we never created.
        let _ = std::fs::remove_dir(dir);
    }
}

/// Delete empty parent directories that this call may have created for a set
/// of touched file paths. Purely best-effort: only empty dirs are removed.
fn cleanup_parents(paths: &[PathBuf]) {
    for path in paths {
        // Only remove the immediate parent if this call created it;
        // remove_dir fails harmlessly when the dir is not empty.
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Roll back every recorded entry (oldest first), recording which restores
/// succeeded and which failed. Best-effort cleanup of created directory
/// chains afterwards.
fn rollback(
    prior: &[(PathBuf, &Prior)],
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for (path, prior_state) in prior {
        match prior_state.restore(path) {
            Ok(()) => restored.push(path.clone()),
            Err(_) => failed.push(path.clone()),
        }
    }
    if !restored.is_empty() || !failed.is_empty() {
        let paths: Vec<PathBuf> = prior.iter().map(|(p, _)| p.clone()).collect();
        cleanup_parents(&paths);
    }
    (restored, failed)
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

    // Recorded state of files touched so far by this call (for rollback).
    let mut prior: Vec<(PathBuf, Prior)> = Vec::with_capacity(files.len());

    for (path, content) in files {
        // Create the parent directory tree when needed.
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let list: Vec<(PathBuf, &Prior)> =
                    prior.iter().map(|(p, s)| (p.clone(), s)).collect();
                let (restored, failed) = rollback(&list);
                // create_dir_all may have partially created ancestor dirs;
                // clean up the chain (only empty dirs are removed).
                cleanup_dir_chain(parent);
                return if failed.is_empty() {
                    Err(ApplyError::WriteFailed {
                        path: path.to_path_buf(),
                        message: format!("cannot create parent directory: {e}"),
                    })
                } else {
                    Err(ApplyError::RollbackIncomplete { restored, failed })
                };
            }
        }

        // Capture the prior state before mutating the tree.
        let prior_state = if path.exists() {
            match std::fs::read(path) {
                Ok(bytes) => Prior::Existing(bytes),
                Err(e) => {
                    // Cannot back up an existing file: treat as un-writable.
                    let list: Vec<(PathBuf, &Prior)> =
                        prior.iter().map(|(p, s)| (p.clone(), s)).collect();
                    let (restored, failed) = rollback(&list);
                    return if failed.is_empty() {
                        Err(ApplyError::WriteFailed {
                            path: path.to_path_buf(),
                            message: format!("cannot read existing file: {e}"),
                        })
                    } else {
                        Err(ApplyError::RollbackIncomplete { restored, failed })
                    };
                }
            }
        } else {
            Prior::Created
        };

        // Perform the write.
        if let Err(e) = std::fs::write(path, content.as_bytes()) {
            // This path was NOT written, so it belongs to no restore list;
            // but its parent chain may have been created by this call, so
            // clean it up too (only empty dirs are removed).
            let list: Vec<(PathBuf, &Prior)> =
                prior.iter().map(|(p, s)| (p.clone(), s)).collect();
            let (restored, failed) = rollback(&list);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                cleanup_dir_chain(parent);
            }
            return if failed.is_empty() {
                Err(ApplyError::WriteFailed {
                    path: path.to_path_buf(),
                    message: format!("write failed: {e}"),
                })
            } else {
                Err(ApplyError::RollbackIncomplete { restored, failed })
            };
        }

        prior.push((path.to_path_buf(), prior_state));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Visible tests live in tests/steps.rs (installed by setup.sh).
}

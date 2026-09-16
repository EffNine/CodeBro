//! Atomic multi-file writes (eval fixture). See `spec.md`.
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum ApplyError {
    WriteFailed { path: PathBuf, message: String },
    RollbackIncomplete { restored: Vec<PathBuf>, failed: Vec<PathBuf> },
}

pub fn apply_batch(files: &[(PathBuf, String)]) -> Result<(), ApplyError> {
    if files.is_empty() {
        return Ok(());
    }

    // (path, prior) where prior = Some(bytes) for pre-existing files,
    // None for files created by this call.
    let mut written: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::with_capacity(files.len());

    for (path, content) in files {
        // Capture prior state in memory only — never write extra files.
        let prior = if path.exists() {
            match std::fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(_) => None, // unreadable prior file: treat as created
            }
        } else {
            None
        };

        // Create parent directories if needed.
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    // Nothing was written yet for this entry; roll back all
                    // previously-written entries.
                    return rollback(&written, path, format!(
                        "failed to create parent directory {}: {}",
                        parent.display(), e
                    ));
                }
            }
        }

        match std::fs::write(path, content) {
            Ok(()) => {
                written.push((path.clone(), prior));
            }
            Err(e) => {
                // This entry was NOT written — it is not in `written`.
                // Roll back every file that WAS written by this call.
                return rollback(&written, path, e.to_string());
            }
        }
    }

    Ok(())
}

fn rollback(
    written: &[(PathBuf, Option<Vec<u8>>)],
    failed_path: &PathBuf,
    message: String,
) -> Result<(), ApplyError> {
    if written.is_empty() {
        // Nothing was written — clean failure, nothing to restore.
        return Err(ApplyError::WriteFailed {
            path: failed_path.clone(),
            message,
        });
    }

    let mut restored: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<PathBuf> = Vec::new();

    // Reverse order: most-recent write undone first.
    for i in (0..written.len()).rev() {
        let (path, prior) = &written[i];
        let ok = match prior {
            Some(bytes) => std::fs::write(path, bytes).is_ok(),
            None => {
                // This call created the file — delete it.
                std::fs::remove_file(path).is_ok() || !path.exists()
            }
        };
        if ok {
            restored.push(path.clone());
        } else {
            failed.push(path.clone());
        }
    }

    if failed.is_empty() {
        Err(ApplyError::WriteFailed {
            path: failed_path.clone(),
            message,
        })
    } else {
        Err(ApplyError::RollbackIncomplete {
            restored,
            failed,
        })
    }
}

#[cfg(test)]
mod tests {
    // Visible tests live in tests/steps.rs (installed by setup.sh).
}

//! Atomic multi-file writes (eval fixture). See `spec.md`.
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum ApplyError {
    WriteFailed { path: PathBuf, message: String },
    RollbackIncomplete { restored: Vec<PathBuf>, failed: Vec<PathBuf> },
}

pub fn apply_batch(files: &[(PathBuf, String)]) -> Result<(), ApplyError> {
    let mut backups: Vec<(PathBuf, FileState)> = Vec::with_capacity(files.len());

    for (path, content) in files {
        let state = match std::fs::read(path) {
            Ok(bytes) => FileState::Existing(bytes),
            Err(_) => FileState::Created,
        };

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    let rollback =
                        rollback(&backups, path, &format!("create {}: {}", parent.display(), e));
                    return Err(rollback);
                }
            }
        }

        if let Err(e) = std::fs::write(path, content) {
            let rollback =
                rollback(&backups, path, &format!("write {}: {}", path.display(), e));
            return Err(rollback);
        }

        backups.push((path.clone(), state));
    }

    Ok(())
}

#[derive(Clone)]
enum FileState {
    Existing(Vec<u8>),
    Created,
}

fn rollback(
    backups: &[(PathBuf, FileState)],
    failed_path: &PathBuf,
    fail_message: &str,
) -> ApplyError {
    let mut restored: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<PathBuf> = Vec::new();

    for (path, state) in backups.iter().rev() {
        let ok = match state {
            FileState::Existing(bytes) => std::fs::write(path, bytes).is_ok(),
            FileState::Created => std::fs::remove_file(path).is_ok(),
        };
        if ok {
            restored.push(path.clone());
        } else {
            failed.push(path.clone());
        }
    }

    if failed.is_empty() {
        ApplyError::WriteFailed {
            path: failed_path.clone(),
            message: fail_message.to_string(),
        }
    } else {
        ApplyError::RollbackIncomplete { restored, failed }
    }
}

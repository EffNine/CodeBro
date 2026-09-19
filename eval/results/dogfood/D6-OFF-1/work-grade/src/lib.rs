//! Atomic multi-file writes (eval fixture). See `spec.md`.
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum ApplyError {
    /// A write failed and every earlier effect was cleanly rolled back.
    WriteFailed { path: PathBuf, message: String },
    /// A write failed AND the rollback itself could not complete.
    /// The tree is in an UNCERTAIN state: `restored` lists what was put back,
    /// `failed` lists what could not be.
    RollbackIncomplete { restored: Vec<PathBuf>, failed: Vec<PathBuf> },
}

/// Write every `(path, content)` pair, creating parent directories as needed.
///
/// - Empty input is `Ok(())` and touches nothing.
/// - On the FIRST write failure: restore every file written by this call to
///   its prior content (or delete it if this call created it), then return.
/// - Backups live in memory, never as extra files in the target tree.
pub fn apply_batch(files: &[(PathBuf, String)]) -> Result<(), ApplyError> {
    // Prior state for each unique path touched by this call, in write order:
    //   Some(bytes) -> pre-existing file; rollback must restore these exact bytes
    //   None        -> did not exist; this call created it; rollback must delete it
    let mut prior: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
    let mut written: Vec<PathBuf> = Vec::new();

    let first_failure: Result<(), (PathBuf, String)> = files.iter().try_for_each(|(path, content)| {
        if !prior.contains_key(path) {
            match fs::read(path) {
                Ok(bytes) => {
                    prior.insert(path.clone(), Some(bytes));
                    written.push(path.clone());
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    prior.insert(path.clone(), None);
                    written.push(path.clone());
                }
                Err(e) => {
                    return Err((
                        path.clone(),
                        format!("could not back up pre-existing file: {e}"),
                    ));
                }
            }
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .map_err(|e| (path.clone(), format!("could not create parent directories: {e}")))?;
        }
        fs::write(path, content)
            .map_err(|e| (path.clone(), format!("could not write file: {e}")))?;
        Ok(())
    });

    match first_failure {
        Ok(()) => Ok(()),
        Err((failed_path, message)) => {
            let mut restored = Vec::new();
            let mut failed = Vec::new();
            for path in &written {
                let ok = match &prior[path] {
                    Some(original) => fs::write(path, original).is_ok(),
                    None => fs::remove_file(path).is_ok(),
                };
                if ok {
                    restored.push(path.clone());
                } else {
                    failed.push(path.clone());
                }
            }
            if failed.is_empty() {
                Err(ApplyError::WriteFailed { path: failed_path, message })
            } else {
                Err(ApplyError::RollbackIncomplete { restored, failed })
            }
        }
    }
}

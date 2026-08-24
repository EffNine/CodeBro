//! Atomic file persistence helpers for CodeBro state files.
//!
//! All durable JSON state (facts store, engineering memory, project
//! identity) must be written through [`write_atomic`]: content is staged
//! in a temporary file inside the destination directory, flushed and
//! fsynced, then moved into place with an atomic rename. A crash or
//! disk-full event therefore can never truncate an existing store.
//!
//! Corrupt stores are handled by [`quarantine_file`], which renames the
//! unreadable file aside under a `.corrupt-<timestamp>` suffix. The raw
//! bytes are preserved for manual recovery; nothing is deleted.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Write `contents` to `path` atomically and durably.
///
/// Steps: create parent directory if needed → stage temp file in the same
/// directory (same filesystem, so rename is atomic) → write + flush +
/// fsync the temp file → rename over the destination → fsync the parent
/// directory so the rename itself survives a crash.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    tmp.write_all(contents)?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    // Best-effort directory sync: makes the rename durable on ext4/xfs.
    if let Ok(dir_handle) = fs::File::open(&dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

/// Move a corrupt file aside to `<name>.corrupt-<UTC timestamp>` within
/// its own directory. Returns the quarantine path, or `None` when the
/// source does not exist. Never deletes data.
pub fn quarantine_file(path: &Path) -> io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "state-file".to_string());
    let dest = path.with_file_name(format!("{file_name}.corrupt-{timestamp}"));
    // If two quarantines happen within one second, shift instead of overwrite.
    let mut dest = dest;
    let mut n = 1u32;
    while dest.exists() {
        dest = path.with_file_name(format!("{file_name}.corrupt-{timestamp}.{n}"));
        n += 1;
    }
    fs::rename(path, &dest)?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn atomic_write_creates_replaces_and_leaves_no_residue() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("store.json");

        write_atomic(&target, b"{\"v\":1}").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"{\"v\":1}");

        // Replace existing content through the same path.
        write_atomic(&target, b"{\"v\":2}").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"{\"v\":2}");

        // No temp residue left behind in the directory.
        let residue: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name != "store.json")
            .collect();
        assert!(residue.is_empty(), "unexpected residue: {residue:?}");
    }

    #[test]
    fn atomic_write_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a/b/c/store.json");
        write_atomic(&target, b"ok").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"ok");
    }

    #[test]
    fn quarantine_preserves_bytes_and_clears_original_path() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("memory.json");
        let mut f = fs::File::create(&target).unwrap();
        f.write_all(b"{truncated json payload").unwrap();
        drop(f);

        let quarantined = quarantine_file(&target).unwrap().expect("quarantine path");
        assert!(quarantined.exists(), "quarantined copy must exist");
        assert!(
            quarantined.starts_with(dir.path()),
            "quarantine stays beside the original"
        );
        assert!(
            quarantined
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".corrupt-"),
            "quarantine name carries .corrupt marker"
        );
        // Raw bytes preserved verbatim.
        assert_eq!(fs::read(&quarantined).unwrap(), b"{truncated json payload");
        // Original path cleared so a clean write can proceed.
        assert!(!target.exists());

        // Quarantining a missing file is a no-op returning None.
        assert!(quarantine_file(&target).unwrap().is_none());
    }
}

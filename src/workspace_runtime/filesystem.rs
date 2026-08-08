#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Filesystem Abstraction (P10.4).
//!
//! A thin, thread-safe abstraction over the local filesystem. The Workspace
//! Runtime discovers and observes the workspace ONLY through this layer. It
//! never reaches for raw `std::fs` traversal outside the abstraction, which
//! lets callers stay decoupled from the concrete filesystem.
//!
//! Traversal is **bounded**: a maximum depth and exclusion globs keep
//! walking cheap. This is a listing/observation abstraction — it does no
//! content indexing.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::workspace_runtime::context::{
    WorkspaceContext, WorkspaceRuntimeError, WorkspaceRuntimeResult,
};

/// The kind of a filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

/// A single observed entry. Immutable and cheap to clone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryInfo {
    /// Path relative to the workspace root.
    pub rel_path: PathBuf,
    /// Byte length of a file (`0` for directories).
    pub size: u64,
    /// Last modification time, if known.
    pub modified_ms: Option<u64>,
    pub kind: EntryKind,
}

/// A directory listing result with iteration bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub entries: Vec<EntryInfo>,
    pub depth: usize,
    pub truncated: bool,
}

/// The filesystem abstraction contract.
pub trait FileSystem: Send + Sync {
    /// Whether `root` looks like a usable workspace directory.
    fn is_directory(&self, root: &Path) -> bool;

    /// Bounded shallow traversal of a workspace root.
    ///
    /// This is intentionally lazy: it lists at most `depth` levels and
    /// returns immediately when `max_entries` is reached, so callers can
    /// bound memory and time.
    fn list(&self, ctx: &WorkspaceContext, max_entries: usize) -> WorkspaceRuntimeResult<Listing>;

    /// Read the byte length of a single file.
    fn file_size(&self, path: &Path) -> WorkspaceRuntimeResult<u64>;

    /// Whether a path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Read a small file fully (cap on bytes to stay bounded).
    fn read_small(&self, path: &Path, cap_bytes: usize) -> WorkspaceRuntimeResult<Vec<u8>>;
}

/// Concrete local filesystem implementation.
#[derive(Debug, Default, Clone)]
pub struct LocalFileSystem;

impl LocalFileSystem {
    pub fn new() -> Self {
        LocalFileSystem
    }
}

impl FileSystem for LocalFileSystem {
    fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn list(&self, ctx: &WorkspaceContext, max_entries: usize) -> WorkspaceRuntimeResult<Listing> {
        let root = &ctx.root.0;
        if !root.is_dir() {
            return Ok(Listing {
                entries: Vec::new(),
                depth: 0,
                truncated: false,
            });
        }

        let mut entries = Vec::new();
        let mut truncated = false;
        let mut stack: Vec<(PathBuf, usize)> = vec![(root.clone(), 0)];

        while let Some((dir, depth)) = stack.pop() {
            if ctx.max_depth.map_or(false, |md| depth > md) {
                continue;
            }
            let read = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in read.flatten() {
                if entries.len() >= max_entries {
                    truncated = true;
                    break;
                }
                let path = entry.path();
                let rel = match path.strip_prefix(root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => continue,
                };
                if ctx.is_excluded(&rel) {
                    continue;
                }
                let kind = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    EntryKind::Directory
                } else if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
                    EntryKind::Symlink
                } else {
                    EntryKind::File
                };
                let size = if kind == EntryKind::File {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };
                let modified_ms = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|s| s.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64);
                entries.push(EntryInfo {
                    rel_path: rel.clone(),
                    size,
                    modified_ms,
                    kind,
                });
                if kind == EntryKind::Directory {
                    stack.push((path, depth + 1));
                }
            }
            if truncated {
                break;
            }
        }
        Ok(Listing {
            entries,
            depth: ctx.max_depth.unwrap_or(usize::MAX),
            truncated,
        })
    }

    fn file_size(&self, path: &Path) -> WorkspaceRuntimeResult<u64> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| WorkspaceRuntimeError::Io(e.to_string()))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_small(&self, path: &Path, cap_bytes: usize) -> WorkspaceRuntimeResult<Vec<u8>> {
        use std::io::Read;
        let f = std::fs::File::open(path).map_err(|e| WorkspaceRuntimeError::Io(e.to_string()))?;
        let mut buf = Vec::with_capacity(cap_bytes.min(8192));
        let mut limited = (&f).take(cap_bytes as u64);
        limited
            .read_to_end(&mut buf)
            .map_err(|e| WorkspaceRuntimeError::Io(e.to_string()))?;
        Ok(buf)
    }
}

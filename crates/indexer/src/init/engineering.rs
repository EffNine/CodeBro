//! P6 engineering intelligence: file inventory, incremental diff,
//! index lifecycle, freshness, and architecture observations.
//!
//! ```text
//! Repository (source of truth)
//!   │  discover + hash
//!   ▼
//! FileInventory (derived; path → hash + file intelligence)
//!   │  diff(prev, curr)
//!   ▼
//! FileDiff { added, deleted, modified, unchanged }
//!   │  reparse affected only (parse cache) + rebuild model
//!   ▼
//! IndexFreshness { status, indexed_at, revision, counts, stale_count }
//! ```
//!
//! Design notes:
//! - The repository is canonical; everything here is derived state.
//! - Full file *contents* are never stored — hashes only.
//! - Symbol IDs are deterministic (`sym::<rel>::…`), so unchanged files
//!   yield byte-identical records across reindexes (preserved, not rewritten).
//! - Deleted files leave no orphaned graph state: the model is rebuilt
//!   from the current file list, so removed paths simply disappear.
//! - No scheduler/daemon/watcher: all functions are request-driven and
//!   pure over explicit inputs.

#![allow(dead_code, unused_imports)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Index lifecycle ────────────────────────────────────────────────────

/// Request-driven index lifecycle. No background transitions exist:
/// every state change is caused by an explicit caller (`reindex`,
/// `init`, or a failed attempt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IndexStatus {
    Unknown,
    Discovering,
    Indexing,
    Ready,
    Stale,
    Failed,
}

impl IndexStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            IndexStatus::Unknown => "UNKNOWN",
            IndexStatus::Discovering => "DISCOVERING",
            IndexStatus::Indexing => "INDEXING",
            IndexStatus::Ready => "READY",
            IndexStatus::Stale => "STALE",
            IndexStatus::Failed => "FAILED",
        }
    }

    pub fn parse(s: &str) -> Option<IndexStatus> {
        match s {
            "UNKNOWN" => Some(IndexStatus::Unknown),
            "DISCOVERING" => Some(IndexStatus::Discovering),
            "INDEXING" => Some(IndexStatus::Indexing),
            "READY" => Some(IndexStatus::Ready),
            "STALE" => Some(IndexStatus::Stale),
            "FAILED" => Some(IndexStatus::Failed),
            _ => None,
        }
    }
}

impl std::fmt::Display for IndexStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Incremental diff ─────────────────────────────────────────────────

/// Deterministic incremental diff between two file-digest maps
/// (workspace-relative path → SHA-256 hex). All four lists are sorted.
///
/// This is the pure kernel underlying the existing
/// [`super::compute_facts_diff`] pipeline (which adds filesystem I/O +
/// impact projection over the frozen store). Kept as a free function so
/// unit tests, MCP `reindex`, and freshness can share one deterministic
/// implementation without duplicating traversal logic.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FileDiff {
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub modified: Vec<String>,
    pub unchanged: Vec<String>,
}

impl FileDiff {
    /// True when nothing changed.
    pub fn is_clean(&self) -> bool {
        self.added.is_empty() && self.deleted.is_empty() && self.modified.is_empty()
    }

    /// Files that require reparsing (added + modified), sorted.
    pub fn needs_reparse(&self) -> Vec<String> {
        let mut out = self.added.clone();
        out.extend(self.modified.iter().cloned());
        out.sort();
        out.dedup();
        out
    }

    pub fn changed_count(&self) -> usize {
        self.added.len() + self.deleted.len() + self.modified.len()
    }
}

/// Compute the diff between previous (`prev`) and current (`curr`)
/// digest maps. Pure, deterministic, no I/O.
pub fn diff_digests(prev: &BTreeMap<String, String>, curr: &BTreeMap<String, String>) -> FileDiff {
    let mut added = Vec::new();
    let mut deleted = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = Vec::new();
    for (path, curr_hash) in curr {
        match prev.get(path) {
            None => added.push(path.clone()),
            Some(prev_hash) if prev_hash == curr_hash => unchanged.push(path.clone()),
            Some(_) => modified.push(path.clone()),
        }
    }
    for path in prev.keys() {
        if !curr.contains_key(path) {
            deleted.push(path.clone());
        }
    }
    added.sort();
    deleted.sort();
    modified.sort();
    unchanged.sort();
    FileDiff {
        added,
        deleted,
        modified,
        unchanged,
    }
}

// ── Freshness ────────────────────────────────────────────────────────

/// Derived freshness snapshot for an index. Never claims READY when
/// required data is stale: `status` is computed from explicit inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFreshness {
    pub status: IndexStatus,
    /// Unix seconds when the index was produced.
    pub indexed_at: u64,
    /// Repository revision at generation time (working-tree hash or
    /// commit SHA; `"non-git"` when VCS is unavailable).
    pub repository_revision: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
    /// Files in the diff that are not reflected in the index.
    pub stale_count: usize,
}

impl IndexFreshness {
    /// Freshness for a model with no generation state: UNKNOWN, never READY.
    pub fn unknown() -> Self {
        IndexFreshness {
            status: IndexStatus::Unknown,
            indexed_at: 0,
            repository_revision: "unknown".to_string(),
            file_count: 0,
            symbol_count: 0,
            edge_count: 0,
            stale_count: 0,
        }
    }
}

/// Compute freshness of a persisted model against the live repository.
///
/// - `generation_state`: the model's recorded `generation_repo_state`.
/// - `current_state`: live `RepoState::capture(root)`.
/// - `diff`: incremental diff between recorded `file_digests` and the
///   current digest scan (stale files = added+deleted+modified).
/// - `indexed_at`: mtime of `facts.json` (0 when unknown).
pub fn compute_freshness(
    generation_state: Option<&codebro_core::RepoState>,
    current_state: Option<&codebro_core::RepoState>,
    diff: &FileDiff,
    indexed_at: u64,
    file_count: usize,
    symbol_count: usize,
    edge_count: usize,
) -> IndexFreshness {
    let stale_count = diff.changed_count();
    let repository_revision = generation_state
        .map(|s| s.working_tree_hash.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let status = match (generation_state, current_state) {
        (Some(prev), Some(cur)) if prev.working_tree_hash == cur.working_tree_hash => {
            if stale_count == 0 {
                IndexStatus::Ready
            } else {
                IndexStatus::Stale
            }
        }
        (Some(_), Some(_)) => IndexStatus::Stale,
        // Non-git repositories: file digests are the revision signal.
        (None, _) | (_, None) => {
            if stale_count == 0 && file_count > 0 {
                IndexStatus::Ready
            } else if file_count == 0 {
                IndexStatus::Unknown
            } else {
                IndexStatus::Stale
            }
        }
    };
    IndexFreshness {
        status,
        indexed_at,
        repository_revision,
        file_count,
        symbol_count,
        edge_count,
        stale_count,
    }
}

// ── File inventory scan ──────────────────────────────────────────────

/// Scan the working tree for current file digests (path → SHA-256),
/// honouring the same skip-list and size gate as the init pipeline.
/// Returns the digest map; oversized/unreadable files are skipped and
/// counted via `skipped`.
pub fn scan_current_digests(root: &Path) -> (BTreeMap<String, String>, usize) {
    use walkdir::WalkDir;
    const MAX_SOURCE_FILE_BYTES: u64 = 512 * 1024;
    let mut digests = BTreeMap::new();
    let mut skipped = 0usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !super::repo_intel::skip_dir(&e.file_name().to_string_lossy()))
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        // Only track files the index cares about: supported source
        // extensions at file level (parsed + file-level-only) plus
        // well-known manifests. Everything else (images, binaries,
        // lockfiles excluded) stays out of the freshness comparison so
        // unrelated asset churn never marks the engineering index stale.
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let name = entry.file_name().to_string_lossy().to_string();
        let rel_probe = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy();
        let file_level =
            crate::intelligence::file_classify::classify(rel_probe.as_ref(), &name, None);
        let known_ext =
            crate::intelligence::parser::languages::file_language_from_extension(ext).is_some();
        let known_manifest = matches!(
            name.as_str(),
            "Cargo.toml"
                | "go.mod"
                | "package.json"
                | "pyproject.toml"
                | "setup.py"
                | "setup.cfg"
                | "requirements.txt"
        );
        if !known_ext && !known_manifest && file_level.is_empty() {
            continue;
        }
        if std::fs::metadata(entry.path())
            .map(|m| m.len())
            .unwrap_or(u64::MAX)
            > MAX_SOURCE_FILE_BYTES
        {
            skipped += 1;
            continue;
        }
        let Ok(content) = std::fs::read(entry.path()) else {
            skipped += 1;
            continue;
        };
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        let hash = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&content);
            format!("{:x}", h.finalize())
        };
        digests.insert(rel, hash);
    }
    (digests, skipped)
}

// ── Architecture observations ────────────────────────────────────────

/// Deterministic architecture observation grounded in repository
/// structure. Never LLM-generated; every field cites concrete paths or
/// counts. These are observations, not bugs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureFacts {
    /// Top-level source areas (`src/<area>` or root dirs), sorted.
    pub major_directories: Vec<String>,
    /// Package/crate names, sorted.
    pub packages: Vec<String>,
    /// Build systems detected (`cargo`, `go`, `npm`, …), sorted.
    pub build_systems: Vec<String>,
    /// Entry-point paths, sorted, bounded.
    pub entry_points: Vec<String>,
    /// Config file paths, sorted, bounded.
    pub config_files: Vec<String>,
    /// Test directory paths, sorted.
    pub test_directories: Vec<String>,
    /// Deterministic one-line summary (same template as init).
    pub summary: String,
}

impl ArchitectureFacts {
    pub fn empty() -> Self {
        ArchitectureFacts {
            major_directories: Vec::new(),
            packages: Vec::new(),
            build_systems: Vec::new(),
            entry_points: Vec::new(),
            config_files: Vec::new(),
            test_directories: Vec::new(),
            summary: String::new(),
        }
    }
}

const ARCH_BOUND: usize = 32;

/// Collect architecture facts from the live filesystem + persisted
/// model counts. Bounded (`ARCH_BOUND` per list), deterministic
/// (sorted), evidence-grounded (paths that exist).
pub fn collect_architecture(
    root: &Path,
    model: &crate::engineering_facts::FactsModel,
    summary: &str,
) -> ArchitectureFacts {
    let mut major = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if super::repo_intel::skip_dir(&name) {
                continue;
            }
            if entry.path().is_dir() {
                major.insert(name);
            }
        }
    }
    // Source areas from module paths refine the top-level listing.
    for m in model.modules() {
        if let Some(path) = m.path.as_deref() {
            let area = path
                .strip_prefix("src/")
                .unwrap_or(path)
                .split('/')
                .next()
                .unwrap_or(path)
                .to_string();
            if !area.is_empty() && area != "(root)" {
                major.insert(area);
            }
        }
    }
    let mut packages: Vec<String> = model.packages().iter().map(|p| p.name.clone()).collect();
    packages.sort();
    packages.dedup();

    let mut build_systems = BTreeSet::new();
    for probe in [
        ("Cargo.toml", "cargo"),
        ("go.mod", "go"),
        ("package.json", "npm"),
        ("pyproject.toml", "python"),
        ("setup.py", "python"),
        ("requirements.txt", "python"),
    ] {
        if root.join(probe.0).exists() {
            build_systems.insert(probe.1.to_string());
        }
    }
    let mut entry_points: Vec<String> = model
        .entry_points()
        .iter()
        .map(|e| e.path.clone())
        .collect();
    entry_points.sort();
    entry_points.dedup();
    entry_points.truncate(ARCH_BOUND);

    let mut config_files = Vec::new();
    for probe in [
        "Cargo.toml",
        "go.mod",
        "package.json",
        "pyproject.toml",
        "tsconfig.json",
        ".github/workflows",
    ] {
        if root.join(probe).exists() {
            config_files.push(probe.to_string());
        }
    }
    config_files.sort();
    config_files.truncate(ARCH_BOUND);

    let mut test_dirs = BTreeSet::new();
    for m in model.modules() {
        if let Some(path) = m.path.as_deref() {
            if path.contains("test") {
                if let Some((dir, _)) = path.rsplit_once('/') {
                    test_dirs.insert(dir.to_string());
                }
            }
        }
    }
    let mut test_directories: Vec<String> = test_dirs.into_iter().collect();
    test_directories.sort();
    test_directories.truncate(ARCH_BOUND);

    let mut major_directories: Vec<String> = major.into_iter().collect();
    major_directories.sort();
    major_directories.truncate(ARCH_BOUND);

    ArchitectureFacts {
        major_directories,
        packages,
        build_systems: build_systems.into_iter().collect(),
        entry_points,
        config_files,
        test_directories,
        summary: summary.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn diff_detects_added_deleted_modified_unchanged() {
        let prev = maps(&[("a.rs", "h1"), ("b.rs", "h2"), ("c.rs", "h3")]);
        let curr = maps(&[("a.rs", "h1"), ("b.rs", "H2"), ("d.rs", "h4")]);
        let diff = diff_digests(&prev, &curr);
        assert_eq!(diff.added, vec!["d.rs".to_string()]);
        assert_eq!(diff.deleted, vec!["c.rs".to_string()]);
        assert_eq!(diff.modified, vec!["b.rs".to_string()]);
        assert_eq!(diff.unchanged, vec!["a.rs".to_string()]);
        assert!(!diff.is_clean());
        assert_eq!(
            diff.needs_reparse(),
            vec!["b.rs".to_string(), "d.rs".to_string()]
        );
    }

    #[test]
    fn diff_is_clean_when_identical() {
        let m = maps(&[("a.rs", "h1")]);
        let diff = diff_digests(&m, &m);
        assert!(diff.is_clean());
        assert_eq!(diff.changed_count(), 0);
    }

    #[test]
    fn diff_is_deterministic_and_sorted() {
        let prev = maps(&[]);
        let curr = maps(&[("z.rs", "1"), ("a.rs", "2"), ("m.rs", "3")]);
        let diff = diff_digests(&prev, &curr);
        assert_eq!(
            diff.added,
            vec!["a.rs".to_string(), "m.rs".to_string(), "z.rs".to_string()]
        );
    }

    #[test]
    fn deleted_files_leave_no_orphans_in_diff() {
        // Deletion is explicit: the path appears in `deleted` exactly once
        // and never in any other list.
        let prev = maps(&[("gone.rs", "h")]);
        let curr = maps(&[]);
        let diff = diff_digests(&prev, &curr);
        assert_eq!(diff.deleted, vec!["gone.rs".to_string()]);
        assert!(diff.added.is_empty() && diff.modified.is_empty() && diff.unchanged.is_empty());
    }

    #[test]
    fn hash_change_marks_modified() {
        let prev = maps(&[("f.rs", "aaa")]);
        let curr = maps(&[("f.rs", "aab")]);
        let diff = diff_digests(&prev, &curr);
        assert_eq!(diff.modified, vec!["f.rs".to_string()]);
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn freshness_unknown_without_state() {
        let diff = FileDiff::default();
        let f = compute_freshness(None, None, &diff, 0, 0, 0, 0);
        assert_eq!(f.status, IndexStatus::Unknown);
    }

    #[test]
    fn freshness_ready_when_hashes_match_and_no_stale_files() {
        let state = codebro_core::RepoState {
            commit_sha: "abc".to_string(),
            working_tree_dirty: false,
            working_tree_hash: "h".to_string(),
        };
        let diff = FileDiff::default();
        let f = compute_freshness(Some(&state), Some(&state), &diff, 99, 3, 10, 5);
        assert_eq!(f.status, IndexStatus::Ready);
        assert_eq!(f.indexed_at, 99);
    }

    #[test]
    fn freshness_stale_when_hashes_differ() {
        let a = codebro_core::RepoState {
            commit_sha: "a".to_string(),
            working_tree_dirty: false,
            working_tree_hash: "ha".to_string(),
        };
        let b = codebro_core::RepoState {
            commit_sha: "b".to_string(),
            working_tree_dirty: true,
            working_tree_hash: "hb".to_string(),
        };
        let diff = FileDiff::default();
        let f = compute_freshness(Some(&a), Some(&b), &diff, 1, 1, 1, 1);
        assert_eq!(f.status, IndexStatus::Stale);
    }

    #[test]
    fn freshness_never_ready_with_stale_files() {
        let state = codebro_core::RepoState {
            commit_sha: "abc".to_string(),
            working_tree_dirty: false,
            working_tree_hash: "h".to_string(),
        };
        let diff = FileDiff {
            modified: vec!["x.rs".to_string()],
            ..Default::default()
        };
        let f = compute_freshness(Some(&state), Some(&state), &diff, 1, 2, 2, 2);
        assert_eq!(f.status, IndexStatus::Stale);
        assert_eq!(f.stale_count, 1);
    }

    #[test]
    fn index_status_round_trips() {
        for s in [
            IndexStatus::Unknown,
            IndexStatus::Discovering,
            IndexStatus::Indexing,
            IndexStatus::Ready,
            IndexStatus::Stale,
            IndexStatus::Failed,
        ] {
            assert_eq!(IndexStatus::parse(s.as_str()), Some(s));
        }
    }
}

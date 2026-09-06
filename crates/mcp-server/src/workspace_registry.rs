//! Per-workspace runtime state and a registry that maps canonical roots to
//! independent state objects.
//!
//! A single CodeBro MCP server process may serve multiple workspaces at once.
//! Each workspace has its own:
//! - Fact store (cached by mtime, reloaded when `.codebro/facts.json` changes)
//! - Mutation lock (serializes `apply_change` / `apply_changes` / `record_memory`
//!   / `delete_memory` / `update_identity` / `reindex` for that workspace only)
//! - Recent-edits ring (session context for RCA correlation, per-workspace)
//! - Last-RCA cache (debugging inject context, per-workspace)
//! - Journal lock (serializes evidence journal read-modify-write, per-workspace)
//!
//! Process-global state that is NOT per-workspace:
//! - `SandboxRuntime` (backend selection — local vs OpenSandbox — is a process
//!   setting, not a workspace setting)
//! - MCP tool router (protocol metadata)
//! - Immutable configuration (provider URLs, API keys)
//!
//! # Concurrency
//!
//! The registry uses a read-write lock so concurrent readers (read-only tools)
//! never block each other. Writers (first-open of a new workspace) acquire the
//! write lock; once a `WorkspaceState` is in the map, all subsequent lookups
//! are read-lock fast. `WorkspaceState` fields are either `Arc<Mutex<>>` or
//! immutable, so the registry holds no fine-grained locks itself.
//!
//! # Canonical identity
//!
//! Workspace roots are canonicalized (symlinks resolved, trailing slashes
//! removed) before insertion or lookup. `/repo`, `/repo/`, and
//! `/path/to/../repo` all resolve to the same entry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

/// Cached fact store entry: optional mtime plus the parsed store.
type FactsCacheEntry = (Option<std::time::SystemTime>, FactStore);

use crate::debugging::types::RootCauseAnalysis;
use crate::fact_store::FactStore;

/// One recently applied change, with the tests the advisory recommended for
/// it (the precise causal hint used during failure correlation).
#[derive(Clone)]
pub(crate) struct RecentEdit {
    pub path: String,
    pub at: std::time::Instant,
    pub recommended_tests: Vec<String>,
}

impl std::fmt::Debug for RecentEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecentEdit")
            .field("path", &self.path)
            .field("at", &"Instant")
            .field("recommended_tests", &self.recommended_tests)
            .finish()
    }
}

/// Runtime state owned by a single workspace. All mutation, caching, and
/// debugging surfaces below are scoped to exactly one `canonical_root`.
#[derive(Clone, Debug)]
pub struct WorkspaceState {
    /// Canonical absolute path to the workspace root. Never contains `..`
    /// components and always resolved through the filesystem.
    pub canonical_root: PathBuf,
    /// Cached fact store keyed by `.codebro/facts.json` mtime. Replaced only
    /// when the file's mtime changes — a concurrent `codebro init` is picked
    /// up, but steady-state agent sessions do not re-parse on every call.
    pub facts_cache: Arc<std::sync::Mutex<Option<FactsCacheEntry>>>,
    /// Serializes mutating tool calls within this workspace. See the
    /// `CodeBroMcpServer` doc comment for the full invariant.
    pub mutation_lock: Arc<tokio::sync::Mutex<()>>,
    /// Ring of recently applied changes (path, instant, recommended test
    /// names), capped and pruned, used to correlate execution failures with
    /// this session's edits. In-memory only — never persisted.
    pub(crate) recent_edits: Arc<Mutex<Vec<RecentEdit>>>,
    /// Most recent root-cause analysis from a failing sandbox run, kept in
    /// memory only so consult mode=debugging can inject it as context.
    pub last_rca: Arc<Mutex<Option<RootCauseAnalysis>>>,
    /// Serializes journal read-modify-write cycles within this process.
    /// Cross-process writers rely on the documented single-writer assumption
    /// plus atomic rename (last complete write wins).
    pub journal_lock: Arc<Mutex<()>>,
}

impl WorkspaceState {
    /// Create a fresh workspace state for the given canonical root.
    pub fn new(canonical_root: PathBuf) -> Self {
        Self {
            canonical_root,
            facts_cache: Arc::new(std::sync::Mutex::new(None)),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            recent_edits: Arc::new(Mutex::new(Vec::new())),
            last_rca: Arc::new(Mutex::new(None)),
            journal_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Load (or return the cached copy of) the fact store for this workspace.
    /// The cache is keyed by the mtime of `.codebro/facts.json`; a concurrent
    /// `codebro init` bumps the mtime and forces a reload.
    pub(crate) fn fact_store(&self) -> FactStore {
        let path = self.canonical_root.join(".codebro/facts.json");
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();

        let mut guard = self.facts_cache.lock().expect("facts cache lock");
        if let Some((cached_mtime, store)) = guard.as_ref() {
            if *cached_mtime == mtime {
                return store.clone();
            }
        }
        let store = match std::fs::read(&path) {
            Ok(bytes) => {
                match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
                    Ok(model) => FactStore::from_model(&model),
                    Err(e) => {
                        let quarantined = crate::persistence::quarantine_file(&path).ok().flatten();
                        tracing::warn!(
                            "quarantining unparseable {}: {e} (moved to {})",
                            path.display(),
                            quarantined
                                .as_ref()
                                .map(|q| q.display().to_string())
                                .unwrap_or_else(|| "<quarantine failed>".to_string())
                        );
                        FactStore::empty()
                    }
                }
            }
            Err(_) => FactStore::empty(),
        };
        *guard = Some((mtime, store.clone()));
        store
    }

    /// Invalidate the mtime-based fact store cache so the next call reloads
    /// the freshly written `.codebro/facts.json`. Called after `reindex`.
    pub(crate) fn invalidate_facts_cache(&self) {
        let mut guard = self.facts_cache.lock().expect("facts cache lock");
        *guard = None;
    }

    /// Snapshot the session's recent edits as debugging input.
    pub(crate) fn recent_edits_snapshot(
        &self,
    ) -> Vec<crate::debugging::candidates::RecentEditInput> {
        self.recent_edits
            .lock()
            .expect("recent edits lock")
            .iter()
            .map(|e| crate::debugging::candidates::RecentEditInput {
                path: e.path.clone(),
                seconds_ago: e.at.elapsed().as_secs(),
                recommended_tests: e.recommended_tests.clone(),
            })
            .collect()
    }

    /// Record a successfully applied change for failure correlation.
    pub(crate) fn remember_edit(&self, path: &str, recommended_tests: Vec<String>) {
        const MAX_RECENT_EDITS: usize = 50;
        const RELEVANCE_WINDOW_SECS: u64 = 3600;
        let mut guard = self.recent_edits.lock().expect("recent edits lock");
        guard.retain(|e| e.at.elapsed().as_secs() < RELEVANCE_WINDOW_SECS);
        guard.push(RecentEdit {
            path: path.to_string(),
            at: std::time::Instant::now(),
            recommended_tests,
        });
        if guard.len() > MAX_RECENT_EDITS {
            let overflow = guard.len() - MAX_RECENT_EDITS;
            guard.drain(0..overflow);
        }
    }

    /// Stash the latest analysis so consult mode=debugging can inject it.
    pub(crate) fn store_last_rca(&self, rca: &RootCauseAnalysis) {
        *self.last_rca.lock().expect("last rca lock") = Some(rca.clone());
    }
}

/// Thread-safe registry mapping canonical workspace roots to their runtime
/// state. The registry is created once at server boot and shared across all
/// handler invocations via `Arc`.
#[derive(Clone)]
pub struct WorkspaceRegistry {
    inner: Arc<RwLock<HashMap<PathBuf, Arc<WorkspaceState>>>>,
    /// The server's configured default workspace. When a request omits
    /// `workspace_root`, this default is used. Always set at construction
    /// time and never mutated.
    default_root: PathBuf,
}

impl WorkspaceRegistry {
    /// Create a registry anchored at `default_root`. The default workspace is
    /// opened eagerly so the first request is a fast map lookup.
    pub fn new(default_root: PathBuf) -> Self {
        let ws = Arc::new(WorkspaceState::new(default_root.clone()));
        let mut map: HashMap<PathBuf, Arc<WorkspaceState>> = HashMap::new();
        map.insert(default_root.clone(), ws);
        Self {
            inner: Arc::new(RwLock::new(map)),
            default_root,
        }
    }

    /// Resolve a raw path string to a workspace state.
    ///
    /// - Empty / missing → server default.
    /// - Absolute or relative → canonicalize, then lookup or create.
    /// - Non-existent directory → `Err` (never silently falls back).
    /// - Regular file → `Err`.
    pub fn resolve(&self, raw: Option<&str>) -> Result<Arc<WorkspaceState>, String> {
        let target = match raw {
            None | Some("") => self.default_root.clone(),
            Some(s) => {
                let p = PathBuf::from(s);
                p.canonicalize()
                    .map_err(|e| format!("workspace root '{}' is not usable: {e}", p.display()))?
            }
        };
        if !target.is_dir() {
            return Err(format!(
                "workspace root '{}' is not a directory",
                target.display()
            ));
        }
        Ok(self.get_or_open(&target))
    }

    /// Look up an existing workspace or open a new one. Concurrent callers
    /// requesting the same root for the first time all receive the same
    /// `WorkspaceState` — the write-lock ensures only one is inserted.
    pub fn get_or_open(&self, root: &Path) -> Arc<WorkspaceState> {
        let locked = self.inner.read().unwrap();
        if let Some(ws) = locked.get(root) {
            return ws.clone();
        }
        drop(locked);

        // Need to insert — acquire write lock. Another thread may have inserted
        // in the meantime, so check again inside the write section.
        let mut locked = self.inner.write().unwrap();
        if let Some(ws) = locked.get(root) {
            return ws.clone();
        }
        let ws = Arc::new(WorkspaceState::new(root.to_path_buf()));
        locked.insert(root.to_path_buf(), ws.clone());
        ws
    }

    /// Return the server's configured default workspace root.
    pub fn default_root(&self) -> &Path {
        &self.default_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn default_workspace_is_opened_eagerly() {
        let dir = temp_root();
        let reg = WorkspaceRegistry::new(dir.path().to_path_buf());
        let ws = reg.resolve(None).unwrap();
        assert_eq!(ws.canonical_root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn explicit_root_selects_independent_state() {
        let a = temp_root();
        let b = temp_root();
        let reg = WorkspaceRegistry::new(a.path().to_path_buf());
        let ws_a = reg.resolve(None).unwrap();
        let ws_b = reg
            .resolve(Some(b.path().to_string_lossy().as_ref()))
            .unwrap();
        assert_ne!(
            Arc::as_ptr(&ws_a),
            Arc::as_ptr(&ws_b),
            "different roots must yield different states"
        );
        assert_eq!(ws_a.canonical_root, a.path().canonicalize().unwrap());
        assert_eq!(ws_b.canonical_root, b.path().canonicalize().unwrap());
    }

    #[test]
    fn same_canonical_root_deduplicates() {
        let dir = temp_root();
        let canon = dir.path().canonicalize().unwrap();
        let reg = WorkspaceRegistry::new(dir.path().to_path_buf());
        let first = reg.get_or_open(&canon);
        let second = reg.get_or_open(&canon);
        assert_eq!(
            Arc::as_ptr(&first),
            Arc::as_ptr(&second),
            "same canonical root must yield the same WorkspaceState"
        );
    }

    #[test]
    fn nonexistent_root_errors_closed() {
        let reg = WorkspaceRegistry::new(PathBuf::from("/tmp"));
        let err = reg.resolve(Some("/nonexistent-workspace-xyz")).unwrap_err();
        assert!(err.contains("not usable"));
    }

    #[test]
    fn file_as_root_errors_closed() {
        let dir = temp_root();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let reg = WorkspaceRegistry::new(dir.path().to_path_buf());
        let err = reg
            .resolve(Some(file.to_string_lossy().as_ref()))
            .unwrap_err();
        assert!(err.contains("not a directory"));
    }

    #[test]
    fn symlinked_root_resolves_to_target() {
        let dir = temp_root();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = dir.path().join("link");
            symlink(&real, &link).unwrap();
            let reg = WorkspaceRegistry::new(real.clone());
            let via_link = reg.resolve(Some(link.to_string_lossy().as_ref())).unwrap();
            let via_real = reg.resolve(Some(real.to_string_lossy().as_ref())).unwrap();
            assert_eq!(
                Arc::as_ptr(&via_link),
                Arc::as_ptr(&via_real),
                "symlink and real path must deduplicate"
            );
        }
    }

    #[test]
    fn concurrent_first_open_deduplicates() {
        let dir = temp_root();
        let reg = WorkspaceRegistry::new(PathBuf::from("/tmp/nonexistent-default-for-test"));
        // This test verifies that concurrent threads calling get_or_open with
        // the same path get the same Arc pointer. The default root doesn't
        // need to exist for this test — we only exercise get_or_open directly.
        let canon = dir.path().canonicalize().unwrap();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = reg.clone();
                let c = canon.clone();
                std::thread::spawn(move || r.get_or_open(&c))
            })
            .collect();
        let states: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let ptr = Arc::as_ptr(&states[0]);
        for s in &states[1..] {
            assert_eq!(
                Arc::as_ptr(s),
                ptr,
                "all concurrent opens must return the same Arc"
            );
        }
    }
}

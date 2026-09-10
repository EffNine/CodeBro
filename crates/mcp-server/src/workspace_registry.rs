//! Per-workspace runtime state and a registry that maps canonical roots to
//! independent state objects — now with **launch-time root authorization**
//! (P8 security boundary closure, audit finding F3).
//!
//! A single CodeBro MCP server process may serve multiple workspaces at
//! once — but only among roots the **operator** authorized at launch:
//!
//! 1. the server's configured default root (`--root` /
//!    `CODEBRO_WORKSPACE_ROOT` / cwd) is always authorized;
//! 2. additional roots are authorized only by the operator via repeatable
//!    `--allow-root <path>` CLI flags and/or the `CODEBRO_ALLOW_ROOTS`
//!    environment variable (path-list separated by `:` on unix, `;` on
//!    Windows).
//!
//! A per-call `workspace_root` tool argument is **discovery** (which
//! authorized workspace a call addresses) — never **authorization**. The
//! argument must canonicalize to exactly one authorized root; every
//! other value is refused with a bounded semantic error before any
//! filesystem access beyond the canonicalization probe itself. Existence
//! of a directory, git structure, prior observation, task/skill/memory
//! text, or history rows never authorize anything.
//!
//! The authorized set is frozen at server construction and is immutable
//! for the process lifetime: no tool call can widen it. Restart does not
//! broaden authorization — the set is re-derived from launch
//! configuration each start.
//!
//! Authorization is exact-root, not prefix-based: authorizing
//! `/work/repo` does NOT authorize `/work/repo/sub` or `/work/repo2`.
//! Nested and overlapping roots MAY both be authorized deliberately
//! (operator lists both); each authorized root keeps its own
//! `WorkspaceState` and its own stores, so ownership is by exact
//! canonical root. Sub-root authorization never follows from a parent.
//!
//! Canonicalization uses real filesystem semantics (`Path::canonicalize`
//! resolves symlinks, `..`, repeated separators, and alternate
//! representations), so a symlink pointing outside the authorized set
//! cannot smuggle authorization: it canonicalizes to its target, which
//! must itself be authorized. A symlink pointing at an authorized root is
//! accepted as that same root (identical canonical identity).
//!
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
//! - The authorized-root set (launch-time operator configuration)
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
//! removed) before insertion, lookup, or authorization. `/repo`, `/repo/`,
//! and `/path/to/../repo` all resolve to the same entry.
//!
//! # Authorization vs filesystem guarantees (TOCTOU note)
//!
//! Authorization is checked at `resolve()` time against the canonicalized
//! target; the ChangeEngine independently re-validates workspace boundaries
//! at prepare AND apply time (symlink swaps between checks are refused
//! there). Within this registry, authorization compares canonical paths,
//! so `..` traversal, symlink indirection, and alternate path
//! representations all reduce to the same canonical-root comparison —
//! there is no string-prefix check to fool. A path swapped to a symlink
//! after authorization still canonicalizes to its (unauthorized or
//! authorized) target, and the target's authorization governs.

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

/// The operator-authorized workspace roots for one server process.
///
/// Built at launch from the server's default root plus optional explicit
/// `--allow-root` flags / the `CODEBRO_ALLOW_ROOTS` env var, then frozen.
/// Client tool input can never widen it. Comparison is by canonical path
/// (exact root, never prefix-based).
#[derive(Clone, Debug)]
pub struct AuthorizedRoots {
    canonical: Vec<PathBuf>,
}

impl AuthorizedRoots {
    /// Authorize exactly one root (the single-root model).
    pub fn single(default_root: PathBuf) -> Self {
        Self {
            canonical: vec![default_root],
        }
    }

    /// Authorize the default root plus operator-supplied extras. Extras
    /// that do not exist or are not directories are ignored here (the
    /// launch-time resolver in [`crate::workspace`] logs them); an
    /// authorized root must always be usable.
    pub fn with_extras(default_root: PathBuf, extras: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut canonical = vec![default_root];
        for extra in extras {
            if !canonical.contains(&extra) {
                canonical.push(extra);
            }
        }
        Self { canonical }
    }

    /// Is `canonical_root` (already canonicalized) authorized?
    ///
    /// Exact-match comparison over the frozen set: never prefix-based,
    /// never string-contains. `authorized/parent` being in the set does
    /// not authorize `authorized/parent/sub`.
    pub fn contains(&self, canonical_root: &Path) -> bool {
        self.canonical.iter().any(|r| r == canonical_root)
    }

    /// The number of authorized roots (diagnostics/observability).
    pub fn len(&self) -> usize {
        self.canonical.len()
    }

    /// True when no roots are authorized (never the case in practice —
    /// the constructor always includes the default root).
    pub fn is_empty(&self) -> bool {
        self.canonical.is_empty()
    }

    /// True when only the default root is authorized.
    pub fn is_single(&self) -> bool {
        self.canonical.len() == 1
    }

    /// Iterate the authorized canonical roots.
    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.canonical.iter().map(|p| p.as_path())
    }
}

/// Bounded, host-minimal authorization refusal. The message names the
/// authorization rule and points the operator at the mechanism — it never
/// lists directory contents, never embeds host filesystem details beyond
/// the caller's own input, and never discloses whether other roots are
/// authorized.
pub(crate) fn unauthorized_root_error(raw: &str) -> String {
    format!(
        "workspace root '{raw}' is not authorized for this server; the operator \
         can authorize additional roots at launch with --allow-root <path> \
         (or CODEBRO_ALLOW_ROOTS)"
    )
}

/// Thread-safe registry mapping canonical workspace roots to their runtime
/// state, restricted to a frozen launch-time authorized root set. The
/// registry is created once at server boot and shared across all handler
/// invocations via `Arc`.
#[derive(Clone)]
pub struct WorkspaceRegistry {
    inner: Arc<RwLock<HashMap<PathBuf, Arc<WorkspaceState>>>>,
    /// The server's configured default workspace. When a request omits
    /// `workspace_root`, this default is used. Always set at construction
    /// time and never mutated.
    default_root: PathBuf,
    /// The frozen set of operator-authorized roots (P8 boundary closure).
    /// Every explicit `workspace_root` argument must canonicalize to one
    /// of these; anything else is refused before the workspace is opened.
    authorized: AuthorizedRoots,
}

impl WorkspaceRegistry {
    /// Create a registry anchored at `default_root` with no additional
    /// authorized roots (single-root authorization — the default model).
    /// The default workspace is opened eagerly so the first request is a
    /// fast map lookup.
    pub fn new(default_root: PathBuf) -> Self {
        let authorized = AuthorizedRoots::single(default_root.clone());
        Self::with_authorized_roots(default_root, authorized)
    }

    /// Create a registry with an explicit authorized-root set. The default
    /// root MUST be part of the set (enforced by the constructor contract:
    /// `AuthorizedRoots::with_extras`/`single` always include it).
    pub fn with_authorized_roots(default_root: PathBuf, authorized: AuthorizedRoots) -> Self {
        let ws = Arc::new(WorkspaceState::new(default_root.clone()));
        let mut map: HashMap<PathBuf, Arc<WorkspaceState>> = HashMap::new();
        map.insert(default_root.clone(), ws);
        Self {
            inner: Arc::new(RwLock::new(map)),
            default_root,
            authorized,
        }
    }

    /// Resolve a raw path string to a workspace state.
    ///
    /// - Empty / missing → server default (always authorized).
    /// - Absolute or relative → **canonicalize** (real filesystem
    ///   semantics: resolves symlinks, `..`, alternate representations),
    ///   then **authorize** (exact match against the frozen launch-time
    ///   set), then lookup or open.
    /// - Unauthorized (including any existing directory the operator did
    ///   not authorize) → `Err` before any workspace state is created and
    ///   before any per-tool filesystem access runs.
    /// - Non-existent directory → `Err` (never silently falls back).
    /// - Regular file → `Err`.
    ///
    /// This is the F3 closure seam: existence never implies authorization.
    pub fn resolve(&self, raw: Option<&str>) -> Result<Arc<WorkspaceState>, String> {
        let target = match raw {
            None | Some("") => self.default_root.clone(),
            Some(s) => {
                let p = PathBuf::from(s);
                let canonical = p
                    .canonicalize()
                    .map_err(|e| format!("workspace root '{}' is not usable: {e}", p.display()))?;
                if !canonical.is_dir() {
                    return Err(format!(
                        "workspace root '{}' is not a directory",
                        canonical.display()
                    ));
                }
                // Authorization gate: exact canonical-root membership in
                // the frozen operator set. This must run BEFORE
                // get_or_open so no state (fact caches, mutation locks,
                // .codebro writes) is ever created for an unauthorized
                // root.
                if !self.authorized.contains(&canonical) {
                    return Err(unauthorized_root_error(s));
                }
                canonical
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
    ///
    /// Only reachable with an authorized canonical root (callers go
    /// through [`resolve`](Self::resolve)); the default root inserted at
    /// construction is authorized by definition.
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

    /// The frozen authorized-root set (diagnostics/observability).
    pub fn authorized_roots(&self) -> &AuthorizedRoots {
        &self.authorized
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
        let reg = WorkspaceRegistry::with_authorized_roots(
            a.path().to_path_buf(),
            AuthorizedRoots::with_extras(
                a.path().to_path_buf(),
                [b.path().canonicalize().unwrap()],
            ),
        );
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
            let reg = WorkspaceRegistry::with_authorized_roots(
                real.clone(),
                AuthorizedRoots::with_extras(real.clone(), [real.canonicalize().unwrap()]),
            );
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

    // ── P8 security boundary closure (audit F3) ────────────────────────

    #[test]
    fn unauthorized_existing_directory_is_refused() {
        // The core F3 regression: an existing directory that the operator
        // did not authorize must be refused — existence is discovery, not
        // authorization.
        let authorized = temp_root();
        let victim = temp_root();
        let reg = WorkspaceRegistry::new(authorized.path().to_path_buf());
        let err = reg
            .resolve(Some(victim.path().to_string_lossy().as_ref()))
            .unwrap_err();
        assert!(
            err.contains("not authorized"),
            "refusal must name authorization: {err}"
        );
    }

    #[test]
    fn authorized_extra_root_is_served() {
        let a = temp_root();
        let b = temp_root();
        let reg = WorkspaceRegistry::with_authorized_roots(
            a.path().to_path_buf(),
            AuthorizedRoots::with_extras(
                a.path().to_path_buf(),
                [b.path().canonicalize().unwrap()],
            ),
        );
        let ws = reg
            .resolve(Some(b.path().to_string_lossy().as_ref()))
            .unwrap();
        assert_eq!(ws.canonical_root, b.path().canonicalize().unwrap());
    }

    #[test]
    fn traversal_to_unauthorized_directory_is_refused() {
        let victim = temp_root();
        // The traversal must canonicalize to the victim (an existing
        // directory) and then be refused by the authorization gate.
        // Build it under the same parent as the victim so the `..` lands
        // there.
        let victim_parent = victim.path().parent().unwrap();
        let a_in_victim_parent = victim_parent.join("auth-traversal-a");
        std::fs::create_dir_all(&a_in_victim_parent).unwrap();
        let reg = WorkspaceRegistry::new(a_in_victim_parent.canonicalize().unwrap());
        let traversal = format!(
            "{}/../{}",
            a_in_victim_parent.display(),
            victim.path().file_name().unwrap().to_string_lossy()
        );
        let err = reg.resolve(Some(&traversal)).unwrap_err();
        assert!(err.contains("not authorized"), "got: {err}");
    }

    #[test]
    fn traversal_to_authorized_root_is_accepted() {
        // `..` that lands back on an authorized root is discovery, not
        // escape: canonicalization resolves it before authorization.
        let a = temp_root();
        let sub = a.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let canon = a.path().canonicalize().unwrap();
        let reg = WorkspaceRegistry::new(canon.clone());
        // sub/.. is the root itself — the simplest honest round trip.
        let round_trip = format!("{}/..", sub.display());
        let ws = reg.resolve(Some(&round_trip)).unwrap();
        assert_eq!(ws.canonical_root, canon);
    }

    #[test]
    fn symlink_escape_is_refused_canonical_target_governs() {
        let authorized = temp_root();
        let victim = temp_root();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = authorized.path().join("victim-link");
            symlink(victim.path(), &link).unwrap();
            let reg = WorkspaceRegistry::new(authorized.path().canonicalize().unwrap());
            let err = reg
                .resolve(Some(link.to_string_lossy().as_ref()))
                .unwrap_err();
            assert!(err.contains("not authorized"), "got: {err}");
        }
    }

    #[test]
    fn symlink_to_authorized_root_deduplicates() {
        let a = temp_root();
        let b = temp_root(); // holds the link outside the root
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = b.path().join("root-link");
            symlink(a.path(), &link).unwrap();
            let canon = a.path().canonicalize().unwrap();
            let reg = WorkspaceRegistry::new(canon.clone());
            let via_link = reg.resolve(Some(link.to_string_lossy().as_ref())).unwrap();
            let via_real = reg
                .resolve(Some(a.path().to_string_lossy().as_ref()))
                .unwrap();
            assert_eq!(via_link.canonical_root, canon);
            assert_eq!(
                Arc::as_ptr(&via_link),
                Arc::as_ptr(&via_real),
                "link to the authorized root is the same workspace"
            );
        }
    }

    #[test]
    fn subdirectory_of_authorized_root_is_not_implicitly_authorized() {
        // Authorization is exact-root, not prefix-based.
        let a = temp_root();
        let sub = a.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let reg = WorkspaceRegistry::new(a.path().canonicalize().unwrap());
        let err = reg
            .resolve(Some(sub.to_string_lossy().as_ref()))
            .unwrap_err();
        assert!(err.contains("not authorized"), "got: {err}");
    }

    #[test]
    fn sibling_with_similar_name_is_not_authorized() {
        let a = temp_root();
        let parent = a.path().parent().unwrap();
        let a2 = parent.join(format!(
            "{}2",
            a.path().file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&a2).unwrap();
        let reg = WorkspaceRegistry::new(a.path().canonicalize().unwrap());
        let err = reg
            .resolve(Some(a2.to_string_lossy().as_ref()))
            .unwrap_err();
        assert!(err.contains("not authorized"), "got: {err}");
    }

    #[test]
    fn nested_roots_both_authorized_when_operator_lists_both() {
        // Nested roots MAY both be authorized deliberately; each keeps
        // exact-root ownership.
        let parent = temp_root();
        let outer = parent.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let reg = WorkspaceRegistry::with_authorized_roots(
            outer.canonicalize().unwrap(),
            AuthorizedRoots::with_extras(
                outer.canonicalize().unwrap(),
                [inner.canonicalize().unwrap()],
            ),
        );
        let ws_outer = reg.resolve(Some(outer.to_string_lossy().as_ref())).unwrap();
        let ws_inner = reg.resolve(Some(inner.to_string_lossy().as_ref())).unwrap();
        assert_eq!(ws_outer.canonical_root, outer.canonicalize().unwrap());
        assert_eq!(ws_inner.canonical_root, inner.canonicalize().unwrap());
        assert_ne!(
            Arc::as_ptr(&ws_outer),
            Arc::as_ptr(&ws_inner),
            "nested authorized roots keep distinct states"
        );
    }

    #[test]
    fn authorized_set_is_frozen_and_not_widenable_by_traffic() {
        // Serving the default root, then requesting an unauthorized one,
        // then re-requesting the default — the authorized set never
        // grows; repeated refusals are byte-identical (determinism).
        let a = temp_root();
        let b = temp_root();
        let reg = WorkspaceRegistry::new(a.path().canonicalize().unwrap());
        let _ = reg.resolve(None).unwrap();
        let e1 = reg
            .resolve(Some(b.path().to_string_lossy().as_ref()))
            .unwrap_err();
        let _ = reg.resolve(None).unwrap();
        let e2 = reg
            .resolve(Some(b.path().to_string_lossy().as_ref()))
            .unwrap_err();
        assert_eq!(e1, e2, "authorization refusals must be deterministic");
        assert_eq!(reg.authorized_roots().len(), 1);
    }

    #[test]
    fn unauthorized_refusal_is_bounded_and_host_minimal() {
        let a = temp_root();
        let victim = temp_root();
        let reg = WorkspaceRegistry::new(a.path().canonicalize().unwrap());
        let err = reg
            .resolve(Some(victim.path().to_string_lossy().as_ref()))
            .unwrap_err();
        // Bounded: no directory listings, no host details beyond the
        // caller's own input.
        assert!(err.len() < 400, "refusal must be bounded: {err}");
        assert!(!err.contains(
            victim
                .path()
                .join("victim_file.txt")
                .to_string_lossy()
                .as_ref()
        ));
    }

    #[test]
    fn concurrent_authorized_and_unauthorized_resolution_is_deterministic() {
        let a = temp_root();
        let b = temp_root();
        let reg = Arc::new(WorkspaceRegistry::with_authorized_roots(
            a.path().canonicalize().unwrap(),
            AuthorizedRoots::with_extras(
                a.path().canonicalize().unwrap(),
                [b.path().canonicalize().unwrap()],
            ),
        ));
        let a_str = a.path().to_string_lossy().into_owned();
        let b_str = b.path().to_string_lossy().into_owned();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let r = reg.clone();
                let target = if i % 2 == 0 {
                    a_str.clone()
                } else {
                    b_str.clone()
                };
                std::thread::spawn(move || r.resolve(Some(&target)))
            })
            .collect();
        for h in handles {
            assert!(
                h.join().unwrap().is_ok(),
                "authorized roots must resolve under concurrency"
            );
        }

        let victim = temp_root();
        let v_str = victim.path().to_string_lossy().into_owned();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = reg.clone();
                let t = v_str.clone();
                std::thread::spawn(move || r.resolve(Some(&t)).is_err())
            })
            .collect();
        for h in handles {
            assert!(
                h.join().unwrap(),
                "unauthorized root must be refused under concurrency"
            );
        }
    }
}

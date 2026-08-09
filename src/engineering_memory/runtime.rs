//! `EngineeringMemoryRuntime` — the thin integration layer over `memory_runtime`.
//!
//! Extends the existing `MemoryRuntime` with:
//! - Persistence to `.codebro/engineering_memory.json`.
//! - Project-tier scoping verified against `ProjectIdentityProvider`.
//! - Deterministic task-resolution pipeline.
//!
//! Operations are explicit only: load, record, update, delete, snapshot, resolve.
//! No automatic learning, reflection, or LLM-driven writes.

use std::path::PathBuf;

use super::provider::{EmptyEngineeringMemoryProvider, EngineeringMemoryProvider};
use super::resolver::EngineeringMemoryResolver;
use super::store::{EngineeringMemoryStore, StorageError};
use super::types::{
    EngineeringMemoryEntry, EngineeringMemoryFile, EngineeringMemoryMetadata,
    EngineeringMemoryResolveError,
};
use crate::engineering_context::memory::EngineeringMemoryContext;
use crate::memory_runtime::{MemoryEntry as RuntimeMemoryEntry, MemoryPolicy, MemoryRuntime, MemoryTier};
use crate::project_identity::ProjectIdentityProvider;

/// Errors that can occur during engineering memory operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineeringMemoryError {
    /// Storage error.
    Storage(StorageError),
    /// The memory file belongs to a different workspace.
    WrongProject(String),
    /// Schema version mismatch.
    WrongSchema(String),
    /// Resolution error.
    Resolution(EngineeringMemoryResolveError),
    /// Generic error.
    Generic(String),
}

impl std::fmt::Display for EngineeringMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineeringMemoryError::Storage(e) => write!(f, "storage: {}", e),
            EngineeringMemoryError::WrongProject(root) => {
                write!(f, "wrong project workspace root: {}", root)
            }
            EngineeringMemoryError::WrongSchema(v) => {
                write!(f, "wrong schema version: {}", v)
            }
            EngineeringMemoryError::Resolution(e) => write!(f, "resolution: {}", e),
            EngineeringMemoryError::Generic(msg) => write!(f, "generic: {}", msg),
        }
    }
}

impl std::error::Error for EngineeringMemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EngineeringMemoryError::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StorageError> for EngineeringMemoryError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::WrongWorkspaceRoot(root) => EngineeringMemoryError::WrongProject(root),
            StorageError::WrongSchemaVersion(v) => EngineeringMemoryError::WrongSchema(v),
            _ => EngineeringMemoryError::Storage(e),
        }
    }
}

impl From<EngineeringMemoryResolveError> for EngineeringMemoryError {
    fn from(e: EngineeringMemoryResolveError) -> Self {
        EngineeringMemoryError::Resolution(e)
    }
}

/// The canonical runtime for managing project-tier engineering memory.
///
/// `EngineeringMemoryRuntime` wraps the existing `MemoryRuntime` and adds
/// file persistence and project-scope verification.
#[derive(Debug)]
pub struct EngineeringMemoryRuntime<P: ProjectIdentityProvider> {
    workspace_root: PathBuf,
    store: EngineeringMemoryStore,
    memory_runtime: MemoryRuntime,
    resolver: EngineeringMemoryResolver,
    identity_provider: P,
    entries: Vec<EngineeringMemoryEntry>,
}

impl<P: ProjectIdentityProvider + Clone> EngineeringMemoryRuntime<P> {
    /// Create a new runtime for the given workspace root and identity provider.
    pub fn new(workspace_root: impl AsRef<std::path::Path>, identity: P) -> Self {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let store = EngineeringMemoryStore::new(&workspace_root);
        let memory_runtime = MemoryRuntime::new(MemoryPolicy::default());
        let resolver = EngineeringMemoryResolver::default();
        EngineeringMemoryRuntime {
            workspace_root,
            store,
            memory_runtime,
            resolver,
            identity_provider: identity,
            entries: Vec::new(),
        }
    }

    /// Return the workspace root path.
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Return a reference to the underlying identity provider.
    pub fn identity_provider(&self) -> &P {
        &self.identity_provider
    }

    /// Return the underlying memory runtime (for diagnostics).
    pub fn memory_runtime(&self) -> &MemoryRuntime {
        &self.memory_runtime
    }

    // ── Load ─────────────────────────────────────────────────────────────

    /// Load persisted entries from `.codebro/engineering_memory.json`.
    ///
    /// Rejects files from a different workspace root or with an unknown schema
    /// version without mutating in-memory state.
    pub fn load(&mut self) -> Result<usize, EngineeringMemoryError> {
        let expected_root = self.workspace_root.to_string_lossy().to_string();
        let file = self.store.load(&expected_root)?;

        // Validate schema version.
        if file.schema_version != super::types::CURRENT_SCHEMA_VERSION {
            return Err(EngineeringMemoryError::WrongSchema(file.schema_version));
        }

        // Validate workspace root.
        if file.workspace_root != expected_root {
            return Err(EngineeringMemoryError::WrongProject(file.workspace_root));
        }

        // Do not mutate in-memory state on failure — we already returned above.
        self.entries = file.entries;

        // Sync with the underlying MemoryRuntime (project tier only).
        self.sync_to_runtime();

        Ok(self.entries.len())
    }

    /// Returns true if a memory file exists for this workspace.
    pub fn memory_exists(&self) -> bool {
        self.store.memory_exists()
    }

    // ── Record ───────────────────────────────────────────────────────────

    /// Record a new engineering memory entry.
    ///
    /// The entry is stored at project tier in both the in-memory store and
    /// the underlying `MemoryRuntime`. The file is NOT persisted automatically;
    /// call `persist()` to write to disk.
    pub fn record(&mut self, entry: EngineeringMemoryEntry) -> Result<(), EngineeringMemoryError> {
        if self.entries.iter().any(|e| e.id == entry.id) {
            return Err(EngineeringMemoryError::Generic(format!(
                "entry already exists: {}",
                entry.id
            )));
        }
        self.entries.push(entry.clone());
        self.sync_to_runtime();
        Ok(())
    }

    // ── Update ───────────────────────────────────────────────────────────

    /// Update an existing entry by id.
    ///
    /// Only the `value` field is updated; id, key, and metadata are preserved.
    pub fn update(
        &mut self,
        id: &str,
        new_value: impl Into<String>,
    ) -> Result<(), EngineeringMemoryError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| EngineeringMemoryError::Generic(format!("entry not found: {}", id)))?;
        entry.value = new_value.into();
        entry.record_access();
        self.sync_to_runtime();
        Ok(())
    }

    // ── Delete ───────────────────────────────────────────────────────────

    /// Delete an entry by id.
    pub fn delete(&mut self, id: &str) -> Result<(), EngineeringMemoryError> {
        let pos = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| EngineeringMemoryError::Generic(format!("entry not found: {}", id)))?;
        self.entries.remove(pos);
        self.sync_to_runtime();
        Ok(())
    }

    // ── Persist ──────────────────────────────────────────────────────────

    /// Persist current in-memory entries to `.codebro/engineering_memory.json`.
    pub fn persist(&self) -> Result<(), EngineeringMemoryError> {
        let file = EngineeringMemoryFile::from_entries(
            self.workspace_root.to_string_lossy().to_string(),
            self.entries.clone(),
        );
        self.store.save(&file)?;
        Ok(())
    }

    // ── Snapshot ─────────────────────────────────────────────────────────

    /// Return a snapshot of the current in-memory entries.
    pub fn snapshot(&self) -> Vec<EngineeringMemoryEntry> {
        self.entries.clone()
    }

    // ── Resolve ──────────────────────────────────────────────────────────

    /// Resolve memory entries for a task query.
    ///
    /// Filters by task keywords, active-file tags, and minimum confidence.
    /// Ranks deterministically and enforces budgets.
    pub fn resolve_for_task(
        &self,
        task_keywords: &[String],
        active_file_tags: &[String],
    ) -> EngineeringMemoryContext {
        match self.resolver.resolve(&self.entries, task_keywords, active_file_tags) {
            Ok(context) => context,
            Err(EngineeringMemoryResolveError::NoMatches) => EngineeringMemoryContext::new(),
            Err(e) => {
                tracing::warn!("Engineering memory resolution failed: {}", e);
                EngineeringMemoryContext::new()
            }
        }
    }

    /// Resolve with the resolver's default budgets.
    pub fn resolve_default(
        &self,
        task_keywords: &[String],
        active_file_tags: &[String],
    ) -> EngineeringMemoryContext {
        self.resolve_for_task(task_keywords, active_file_tags)
    }

    // ── Diagnostics ──────────────────────────────────────────────────────

    /// Returns the number of persisted entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns project-aware diagnostics from the identity provider.
    pub fn diagnostics(&self) -> crate::project_identity::ProjectIdentityDiagnostics {
        self.identity_provider.diagnostics()
    }

    /// Returns project-aware statistics from the identity provider.
    pub fn statistics(&self) -> crate::project_identity::ProjectIdentityStatistics {
        self.identity_provider.statistics()
    }

    // ── Internal ─────────────────────────────────────────────────────────

    fn sync_to_runtime(&mut self) {
        // Clear and rebuild the runtime's project-tier entries from our
        // canonical in-memory list. This keeps the runtime and our store in sync.
        let project_entries: Vec<RuntimeMemoryEntry> = self
            .entries
            .iter()
            .map(|e| {
                RuntimeMemoryEntry::new(&e.id, MemoryTier::Project, &e.key, &e.value)
                    .with_metadata(crate::memory_runtime::MemoryMetadata {
                        importance: e.metadata.importance,
                        confidence: e.metadata.confidence,
                        tags: e.metadata.tags.clone(),
                        source: e.metadata.source.clone(),
                        context: None,
                    })
            })
            .collect();

        // We can't clear the runtime directly, so we rebuild by removing old
        // project entries and inserting fresh ones. The runtime is a simple
        // wrapper; for correctness we just recreate it with our entries.
        *self = EngineeringMemoryRuntime {
            workspace_root: self.workspace_root.clone(),
            store: self.store.clone(),
            memory_runtime: MemoryRuntime::new(MemoryPolicy::default()),
            resolver: self.resolver.clone(),
            identity_provider: self.identity_provider.clone(),
            entries: self.entries.clone(),
        };

        for entry in project_entries {
            let _ = self.memory_runtime.create(entry);
        }
    }
}

impl<P: ProjectIdentityProvider + Clone> EngineeringMemoryProvider for EngineeringMemoryRuntime<P> {
    fn provider_name(&self) -> &str {
        "EngineeringMemoryRuntime"
    }

    fn snapshot(&self) -> EngineeringMemoryContext {
        // Return all project-tier entries without filtering.
        let context_entries: Vec<crate::engineering_context::memory::MemoryEntry> = self
            .entries
            .iter()
            .map(|e| crate::engineering_context::memory::MemoryEntry {
                key: e.key.clone(),
                value: e.value.clone(),
                confidence: e.metadata.confidence,
                tier: crate::engineering_context::memory::MemoryTier::Project,
            })
            .collect();
        EngineeringMemoryContext::new().with_entries(context_entries)
    }

    fn resolve_for_task(
        &self,
        task_keywords: &[String],
        active_file_tags: &[String],
    ) -> EngineeringMemoryContext {
        EngineeringMemoryRuntime::resolve_for_task(self, task_keywords, active_file_tags)
    }

    fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Create an empty engineering memory runtime (for testing / stubbing).
#[cfg(test)]
pub fn empty_runtime() -> EngineeringMemoryRuntime<crate::project_identity::ProjectIdentityRuntime> {
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut identity = crate::project_identity::ProjectIdentityRuntime::new(tmp.path());
    let _ = identity.create_minimal("empty", "rust");
    EngineeringMemoryRuntime::new(tmp.path(), identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_memory::types::{EngineeringMemoryEntry, EngineeringMemoryMetadata};
    use crate::project_identity::{ProjectIdentity, ProjectIdentityRuntime};
    use tempfile::TempDir;

    fn make_entry(id: &str, key: &str, value: &str) -> EngineeringMemoryEntry {
        EngineeringMemoryEntry::new(id, key, value)
            .with_metadata(
                EngineeringMemoryMetadata::new()
                    .with_confidence(0.9)
                    .with_importance(0.8)
                    .with_tag("backend"),
            )
    }

    fn setup() -> (EngineeringMemoryRuntime<ProjectIdentityRuntime>, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let mut identity = ProjectIdentityRuntime::new(tmp.path());
        let _ = identity.create_minimal("test-proj", "rust");
        let runtime = EngineeringMemoryRuntime::new(tmp.path(), identity);
        (runtime, tmp)
    }

    #[test]
    fn test_record_and_snapshot() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "language", "rust")).unwrap();
        let snap = runtime.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].key, "language");
    }

    #[test]
    fn test_update_persists_value() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "language", "rust")).unwrap();
        runtime.update("e1", "go").unwrap();
        let snap = runtime.snapshot();
        assert_eq!(snap[0].value, "go");
    }

    #[test]
    fn test_delete_removes_entry() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "language", "rust")).unwrap();
        runtime.delete("e1").unwrap();
        assert_eq!(runtime.entry_count(), 0);
    }

    #[test]
    fn test_delete_missing_entry_fails() {
        let (mut runtime, _tmp) = setup();
        let result = runtime.delete("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_record_fails() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "language", "rust")).unwrap();
        let result = runtime.record(make_entry("e1", "language", "go"));
        assert!(result.is_err());
    }

    #[test]
    fn test_persist_and_reload() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "language", "rust")).unwrap();
        runtime.record(make_entry("e2", "framework", "axum")).unwrap();
        runtime.persist().expect("persist");

        // Create a fresh runtime pointing to the same directory.
        let mut identity = ProjectIdentityRuntime::new(_tmp.path());
        let _ = identity.load().expect("load identity");
        let mut reload = EngineeringMemoryRuntime::new(_tmp.path(), identity);
        let count = reload.load().expect("reload");
        assert_eq!(count, 2);
        assert_eq!(reload.entry_count(), 2);
    }

    #[test]
    fn test_load_wrong_project_rejected() {
        let (mut runtime, _tmp) = setup();
        // Write a file for a different project.
        let other_file = EngineeringMemoryFile::from_entries(
            "/tmp/other-project".to_string(),
            vec![make_entry("e1", "key", "value")],
        );
        runtime.store.save(&other_file).expect("save");

        // Loading into our runtime should fail.
        let result = runtime.load();
        assert!(matches!(result, Err(EngineeringMemoryError::WrongProject(_))));
        // In-memory state must be unchanged.
        assert_eq!(runtime.entry_count(), 0);
    }

    #[test]
    fn test_load_wrong_schema_rejected() {
        let (mut runtime, _tmp) = setup();
        let mut file = EngineeringMemoryFile::from_entries(
            runtime.workspace_root.to_string_lossy().to_string(),
            vec![make_entry("e1", "key", "value")],
        );
        file.schema_version = "9.9.9".to_string();
        runtime.store.save(&file).expect("save");

        let result = runtime.load();
        assert!(matches!(result, Err(EngineeringMemoryError::WrongSchema(_))));
        assert_eq!(runtime.entry_count(), 0);
    }

    #[test]
    fn test_resolve_empty() {
        let (runtime, _tmp) = setup();
        let ctx = runtime.resolve_for_task(&["auth".to_string()], &[]);
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_resolve_with_entries() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "auth_module", "jwt based")).unwrap();
        runtime.record(make_entry("e2", "database", "postgres")).unwrap();
        runtime.persist().expect("persist");

        let mut reload = {
            let mut identity = ProjectIdentityRuntime::new(_tmp.path());
            let _ = identity.load().expect("load identity");
            EngineeringMemoryRuntime::new(_tmp.path(), identity)
        };
        reload.load().expect("reload memory");
        let ctx = reload.resolve_for_task(&["auth".to_string()], &[]);
        assert!(!ctx.is_empty());
        assert_eq!(ctx.entries.len(), 1);
        assert_eq!(ctx.entries[0].key, "auth_module");
    }

    #[test]
    fn test_resolve_filters_by_tag() {
        let (mut runtime, _tmp) = setup();
        runtime
            .record(
                EngineeringMemoryEntry::new("e1", "frontend", "react")
                    .with_metadata(EngineeringMemoryMetadata::new().with_confidence(0.9).with_tag("ui")),
            )
            .unwrap();
        runtime
            .record(
                EngineeringMemoryEntry::new("e2", "backend", "axum")
                    .with_metadata(EngineeringMemoryMetadata::new().with_confidence(0.9).with_tag("ui")),
            )
            .unwrap();
        runtime.persist().expect("persist");

        let mut reload = {
            let mut identity = ProjectIdentityRuntime::new(_tmp.path());
            let _ = identity.load().expect("load identity");
            EngineeringMemoryRuntime::new(_tmp.path(), identity)
        };
        reload.load().expect("reload memory");
        let ctx = reload.resolve_for_task(&[], &["ui".to_string()]);
        assert_eq!(ctx.entries.len(), 2);
    }

    #[test]
    fn test_provider_trait() {
        let (runtime, _tmp) = setup();
        let provider: &dyn EngineeringMemoryProvider = &runtime;
        assert_eq!(provider.provider_name(), "EngineeringMemoryRuntime");
        assert_eq!(provider.entry_count(), 0);
    }

    #[test]
    fn test_memory_never_alters_project_identity() {
        let (mut runtime, _tmp) = setup();
        let before = runtime.identity_provider().snapshot();
        runtime.record(make_entry("e1", "language", "rust")).unwrap();
        runtime.persist().expect("persist");
        let after = runtime.identity_provider().snapshot();
        assert_eq!(before.name, after.name);
        assert_eq!(before.primary_language(), after.primary_language());
    }

    #[test]
    fn test_memory_isolated_between_workspace_roots() {
        let tmp_a = TempDir::new().expect("temp dir a");
        let tmp_b = TempDir::new().expect("temp dir b");

        let mut identity_a = ProjectIdentityRuntime::new(tmp_a.path());
        let _ = identity_a.create_minimal("proj-a", "rust");
        let mut runtime_a = EngineeringMemoryRuntime::new(tmp_a.path(), identity_a);
        runtime_a.record(make_entry("e1", "key", "value-a")).unwrap();
        runtime_a.persist().expect("persist a");

        let mut identity_b = ProjectIdentityRuntime::new(tmp_b.path());
        let _ = identity_b.create_minimal("proj-b", "go");
        let mut runtime_b = EngineeringMemoryRuntime::new(tmp_b.path(), identity_b);
        runtime_b.record(make_entry("e1", "key", "value-b")).unwrap();
        runtime_b.persist().expect("persist b");

        // Reload each and verify isolation.
        let reload_a = {
            let mut id = ProjectIdentityRuntime::new(tmp_a.path());
            let _ = id.load().expect("load identity a");
            let mut r = EngineeringMemoryRuntime::new(tmp_a.path(), id);
            r.load().expect("reload a");
            r
        };
        let reload_b = {
            let mut id = ProjectIdentityRuntime::new(tmp_b.path());
            let _ = id.load().expect("load identity b");
            let mut r = EngineeringMemoryRuntime::new(tmp_b.path(), id);
            r.load().expect("reload b");
            r
        };

        assert_eq!(reload_a.entry_count(), 1);
        assert_eq!(reload_b.entry_count(), 1);
        assert_eq!(reload_a.snapshot()[0].value, "value-a");
        assert_eq!(reload_b.snapshot()[0].value, "value-b");
    }

    #[test]
    fn test_deterministic_resolve_same_inputs() {
        let (mut runtime, _tmp) = setup();
        runtime.record(make_entry("e1", "auth", "jwt")).unwrap();
        runtime.record(make_entry("e2", "database", "postgres")).unwrap();
        runtime.persist().expect("persist");

        let reload = {
            let mut id = ProjectIdentityRuntime::new(_tmp.path());
            let _ = id.load().expect("load identity");
            let mut r = EngineeringMemoryRuntime::new(_tmp.path(), id);
            r.load().expect("reload");
            r
        };

        let ctx1 = reload.resolve_for_task(&["auth".to_string()], &[]);
        let ctx2 = reload.resolve_for_task(&["auth".to_string()], &[]);
        assert_eq!(ctx1.entries, ctx2.entries);
        assert_eq!(ctx1.budget_remaining, ctx2.budget_remaining);
    }

    #[test]
    fn test_provider_trait_substitution() {
        let (runtime, _tmp) = setup();
        let provider: &dyn EngineeringMemoryProvider = &runtime;
        assert_eq!(provider.provider_name(), "EngineeringMemoryRuntime");
        let snap = provider.snapshot();
        assert!(snap.is_empty());
        let resolved = provider.resolve_for_task(&["auth".to_string()], &[]);
        assert!(resolved.is_empty());
    }
}

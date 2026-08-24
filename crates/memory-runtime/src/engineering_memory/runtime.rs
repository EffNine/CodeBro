//! `EngineeringMemoryRuntime` — the thin integration layer over `memory_runtime`.
//!
//! Extends the existing `MemoryRuntime` with:
//! - Persistence to `.codebro/engineering_memory.json`.
//! - Project-tier scoping verified against `ProjectIdentityProvider`.
//! - Deterministic task-resolution pipeline.
//!
//! Operations are explicit only: load, record, update, delete, snapshot, resolve.
//! No automatic learning, reflection, or LLM-driven writes.

use crate::engineering_memory::types::{ConfidenceAdjustment, MemoryStatus};
use std::path::PathBuf;

use super::provider::{EmptyEngineeringMemoryProvider, EngineeringMemoryProvider};
use super::resolver::EngineeringMemoryResolver;
use super::store::{EngineeringMemoryStore, StorageError};
use super::types::{
    EngineeringMemoryEntry, EngineeringMemoryFile, EngineeringMemoryMetadata,
    EngineeringMemoryResolveError,
};
use crate::engineering_memory::memory_context::EngineeringMemoryContext;
use crate::memory_runtime::{
    MemoryEntry as RuntimeMemoryEntry, MemoryPolicy, MemoryRuntime, MemoryTier,
};
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

        // Validate schema version: accept known 1.x schemas so stores
        // written by older releases keep loading (backwards compatibility).
        let accepted = ["1.0.0", super::types::CURRENT_SCHEMA_VERSION];
        if !accepted.contains(&file.schema_version.as_str()) {
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

        // Lifecycle: expire anything past its TTL before it can resolve.
        self.sweep_expired();
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
    pub fn record(
        &mut self,
        entry: EngineeringMemoryEntry,
    ) -> Result<RecordOutcome, EngineeringMemoryError> {
        if self.entries.iter().any(|e| e.id == entry.id) {
            return Err(EngineeringMemoryError::Generic(format!(
                "entry already exists: {}",
                entry.id
            )));
        }
        // Conflict detection: an ACTIVE entry with the same key but a
        // different value is a key conflict. The new entry supersedes it;
        // the prior entry is retained (status = Superseded) for lineage.
        let mut conflicts: Vec<MemoryConflict> = Vec::new();
        let same_key: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.key == entry.key
                    && e.metadata.status == MemoryStatus::Active
                    && e.value != entry.value
            })
            .map(|(i, _)| i)
            .collect();
        for idx in same_key {
            let prior_id = self.entries[idx].id.clone();
            self.entries[idx].metadata.status = MemoryStatus::Superseded;
            conflicts.push(MemoryConflict {
                kind: ConflictKind::KeyReplaced,
                prior_id,
                prior_key: self.entries[idx].key.clone(),
            });
        }

        // Near-duplicate detection across DIFFERENT keys: high token overlap
        // on values suggests the same knowledge stored twice. Same-key
        // different-value conflicts are handled above as key replacement.
        for e in self.entries.iter() {
            if e.key != entry.key
                && e.metadata.status == MemoryStatus::Active
                && e.value != entry.value
                && token_overlap(&e.value, &entry.value) >= 0.7
            {
                conflicts.push(MemoryConflict {
                    kind: ConflictKind::NearDuplicate,
                    prior_id: e.id.clone(),
                    prior_key: e.key.clone(),
                });
            }
        }

        let mut stored = entry.clone();
        stored.metadata.supersedes = conflicts
            .iter()
            .find(|c| matches!(c.kind, ConflictKind::KeyReplaced))
            .map(|c| c.prior_id.clone());
        self.entries.push(stored);
        self.sync_to_runtime();
        Ok(RecordOutcome {
            id: entry.id,
            conflicts,
        })
    }

    /// Mark every expired entry (`expires_at` in the past) as `Expired`.
    /// Returns the number of transitions. Called automatically on load.
    pub fn sweep_expired(&mut self) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut n = 0;
        for e in self.entries.iter_mut() {
            if e.is_expired_at(now) && e.metadata.status == MemoryStatus::Active {
                e.metadata.status = MemoryStatus::Expired;
                n += 1;
            }
        }
        if n > 0 {
            self.sync_to_runtime();
        }
        n
    }

    /// Adjust an entry's confidence with an auditable trail entry. The new
    /// value is clamped to [0.0, 1.0]. Returns the applied adjustment.
    pub fn adjust_confidence(
        &mut self,
        id: &str,
        to: f64,
        reason: Option<String>,
    ) -> Result<ConfidenceAdjustment, EngineeringMemoryError> {
        let to = to.clamp(0.0, 1.0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        const MAX_ADJUSTMENTS: usize = 16;
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| EngineeringMemoryError::Generic(format!("entry not found: {}", id)))?;
        let adjustment = ConfidenceAdjustment {
            at: now,
            from: entry.metadata.confidence,
            to,
            reason,
        };
        entry.metadata.adjustments.push(adjustment.clone());
        if entry.metadata.adjustments.len() > MAX_ADJUSTMENTS {
            let overflow = entry.metadata.adjustments.len() - MAX_ADJUSTMENTS;
            entry.metadata.adjustments.drain(0..overflow);
        }
        entry.metadata.confidence = to;
        entry.record_access();
        self.sync_to_runtime();
        Ok(adjustment)
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

    /// Update an existing entry's value AND its full metadata
    /// (confidence, importance, tags, source). id, key, and created_at are
    /// preserved so the entry keeps its deterministic identity.
    pub fn update_with_metadata(
        &mut self,
        id: &str,
        new_value: impl Into<String>,
        metadata: EngineeringMemoryMetadata,
    ) -> Result<(), EngineeringMemoryError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| EngineeringMemoryError::Generic(format!("entry not found: {}", id)))?;
        entry.value = new_value.into();
        entry.metadata = metadata;
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
        match self
            .resolver
            .resolve(&self.entries, task_keywords, active_file_tags)
        {
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
        let project_entries: Vec<RuntimeMemoryEntry> =
            self.entries
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
        let context_entries: Vec<crate::engineering_memory::memory_context::MemoryEntry> = self
            .entries
            .iter()
            .map(|e| crate::engineering_memory::memory_context::MemoryEntry {
                key: e.key.clone(),
                value: e.value.clone(),
                confidence: e.metadata.confidence,
                tier: crate::engineering_memory::memory_context::MemoryTier::Project,
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
pub fn empty_runtime() -> EngineeringMemoryRuntime<crate::project_identity::ProjectIdentityRuntime>
{
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
        EngineeringMemoryEntry::new(id, key, value).with_metadata(
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
        runtime
            .record(make_entry("e1", "language", "rust"))
            .unwrap();
        let snap = runtime.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].key, "language");
    }

    #[test]
    fn test_update_persists_value() {
        let (mut runtime, _tmp) = setup();
        runtime
            .record(make_entry("e1", "language", "rust"))
            .unwrap();
        runtime.update("e1", "go").unwrap();
        let snap = runtime.snapshot();
        assert_eq!(snap[0].value, "go");
    }

    #[test]
    fn test_delete_removes_entry() {
        let (mut runtime, _tmp) = setup();
        runtime
            .record(make_entry("e1", "language", "rust"))
            .unwrap();
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
        runtime
            .record(make_entry("e1", "language", "rust"))
            .unwrap();
        let result = runtime.record(make_entry("e1", "language", "go"));
        assert!(result.is_err());
    }

    #[test]
    fn test_persist_and_reload() {
        let (mut runtime, _tmp) = setup();
        runtime
            .record(make_entry("e1", "language", "rust"))
            .unwrap();
        runtime
            .record(make_entry("e2", "framework", "axum"))
            .unwrap();
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
    fn test_oversized_entry_persists_and_stays_discoverable() {
        // P1.3 regression: an oversized entry must survive persist → reload →
        // resolve and remain discoverable (bounded) in a fresh runtime.
        let (mut runtime, _tmp) = setup();
        let long_value = "canonical decision: ".to_string() + &"x".repeat(2900);
        runtime
            .record(make_entry(
                "e1",
                "architecture:mutation-boundary",
                &long_value,
            ))
            .unwrap();
        runtime.persist().expect("persist");

        // Fresh runtime: reload from disk, then resolve with exact keywords.
        let mut identity = ProjectIdentityRuntime::new(_tmp.path());
        let _ = identity.load().expect("load identity");
        let mut reload = EngineeringMemoryRuntime::new(_tmp.path(), identity);
        let count = reload.load().expect("reload");
        assert_eq!(count, 1, "oversized entry must survive reload");

        let ctx = reload.resolve_for_task(&["architecture:mutation-boundary".to_string()], &[]);
        assert_eq!(
            ctx.entries.len(),
            1,
            "oversized entry must resolve after reload"
        );
        let got = &ctx.entries[0];
        assert_eq!(got.key, "architecture:mutation-boundary");
        assert_eq!(got.confidence, 0.9);
        assert!(
            got.value
                .ends_with(crate::engineering_memory::resolver::TRUNCATION_MARKER),
            "recovered oversized entry must be explicitly truncated"
        );
        assert!(got.value.starts_with("canonical decision: "));
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
        assert!(matches!(
            result,
            Err(EngineeringMemoryError::WrongProject(_))
        ));
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
        assert!(matches!(
            result,
            Err(EngineeringMemoryError::WrongSchema(_))
        ));
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
        runtime
            .record(make_entry("e1", "auth_module", "jwt based"))
            .unwrap();
        runtime
            .record(make_entry("e2", "database", "postgres"))
            .unwrap();
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
                EngineeringMemoryEntry::new("e1", "frontend", "react").with_metadata(
                    EngineeringMemoryMetadata::new()
                        .with_confidence(0.9)
                        .with_tag("ui"),
                ),
            )
            .unwrap();
        runtime
            .record(
                EngineeringMemoryEntry::new("e2", "backend", "axum").with_metadata(
                    EngineeringMemoryMetadata::new()
                        .with_confidence(0.9)
                        .with_tag("ui"),
                ),
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
        runtime
            .record(make_entry("e1", "language", "rust"))
            .unwrap();
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
        runtime_a
            .record(make_entry("e1", "key", "value-a"))
            .unwrap();
        runtime_a.persist().expect("persist a");

        let mut identity_b = ProjectIdentityRuntime::new(tmp_b.path());
        let _ = identity_b.create_minimal("proj-b", "go");
        let mut runtime_b = EngineeringMemoryRuntime::new(tmp_b.path(), identity_b);
        runtime_b
            .record(make_entry("e1", "key", "value-b"))
            .unwrap();
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
        runtime
            .record(make_entry("e2", "database", "postgres"))
            .unwrap();
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
    // ── Memory V2: lifecycle, expiry, provenance, conflicts ────────────

    #[test]
    fn key_conflict_supersedes_prior_entry_with_lineage() {
        let mut runtime = empty_runtime();
        runtime
            .record(make_entry("e1", "auth:token", "jwt based"))
            .unwrap();
        let outcome = runtime
            .record(make_entry("e2", "auth:token", "oauth2 with rotation"))
            .unwrap();

        assert_eq!(outcome.conflicts.len(), 1);
        assert_eq!(outcome.conflicts[0].kind_str(), "key_replaced");
        assert_eq!(outcome.conflicts[0].prior_id, "e1");

        let snap = runtime.snapshot();
        let e1 = snap.iter().find(|e| e.id == "e1").unwrap();
        assert_eq!(
            e1.metadata.status,
            crate::engineering_memory::types::MemoryStatus::Superseded
        );
        let e2 = snap.iter().find(|e| e.id == "e2").unwrap();
        assert_eq!(e2.metadata.supersedes.as_deref(), Some("e1"));
    }

    #[test]
    fn identical_key_and_value_is_not_a_conflict() {
        let mut runtime = empty_runtime();
        runtime
            .record(make_entry("e1", "build:cmd", "cargo test --workspace"))
            .unwrap();
        let outcome = runtime
            .record(make_entry("e2", "build:cmd", "cargo test --workspace"))
            .unwrap();
        // Same value under the same key is a benign restatement, not a conflict.
        assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    }

    #[test]
    fn expired_entries_are_swept_and_excluded_from_resolution() {
        let mut runtime = empty_runtime();
        // Distinct keys so key-replacement conflicts don't interfere with
        // the lifecycle being tested.
        let mut entry = make_entry("stale", "deploy:old-region", "eu-west-1");
        entry.metadata.expires_at = Some(1); // epoch second 1: long past
        runtime.record(entry).unwrap();
        runtime
            .record(make_entry(
                "fresh",
                "deploy:new-region",
                "us-east-2 (current)",
            ))
            .unwrap();

        let swept = runtime.sweep_expired();
        assert_eq!(swept, 1);
        let swept_snapshot = runtime.snapshot();
        let stale = swept_snapshot.iter().find(|e| e.id == "stale").unwrap();
        assert_eq!(
            stale.metadata.status,
            crate::engineering_memory::types::MemoryStatus::Expired
        );

        // Resolution must not surface the expired entry even though the
        // keyword matches both entries' keys.
        let context = runtime.resolve_for_task(&["deploy".to_string()], &[]);
        // MemoryContext entries carry key/value pairs; join values for checks.
        let joined = context
            .entries
            .iter()
            .map(|m| m.value.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !joined.contains("eu-west-1"),
            "expired entry leaked: {joined}"
        );
        assert!(
            joined.contains("us-east-2"),
            "active entry missing: {joined}"
        );
    }

    #[test]
    fn confidence_adjustments_leave_an_auditable_trail() {
        let mut runtime = empty_runtime();
        runtime
            .record(make_entry("adj", "risk:rollback", "feature-flagged"))
            .unwrap();
        let a1 = runtime
            .adjust_confidence("adj", 0.7, Some("verified in prod".into()))
            .unwrap();
        assert_eq!((a1.from, a1.to), (0.9, 0.7));
        let _ = runtime.adjust_confidence("adj", 1.5, None).unwrap(); // clamped to 1.0

        let adj_snapshot = runtime.snapshot();
        let entry = adj_snapshot.iter().find(|e| e.id == "adj").unwrap();
        assert_eq!(entry.metadata.confidence, 1.0);
        assert_eq!(entry.metadata.adjustments.len(), 2);
        assert_eq!(
            entry.metadata.adjustments[0].reason.as_deref(),
            Some("verified in prod")
        );

        let missing = runtime.adjust_confidence("ghost", 0.5, None);
        assert!(missing.is_err());
    }

    #[test]
    fn near_duplicate_detection_flags_high_overlap_values() {
        let mut runtime = empty_runtime();
        runtime
            .record(make_entry(
                "orig",
                "api:limit",
                "rate limit is 100 requests per minute per token",
            ))
            .unwrap();
        let outcome = runtime
            .record(make_entry(
                "dup",
                "api:limits-doc",
                "rate limit is 100 requests per minute per token, enforced at gateway",
            ))
            .unwrap();
        assert!(
            outcome
                .conflicts
                .iter()
                .any(|c| c.kind_str() == "near_duplicate"),
            "expected near-duplicate conflict, got {:?}",
            outcome.conflicts
        );
    }

    #[test]
    fn pre_v11_memory_store_loads_with_defaults() {
        use crate::engineering_memory::{store::EngineeringMemoryStore, EngineeringMemoryFile};
        let dir = tempfile::tempdir().unwrap();
        let codebro_dir = dir.path().join(".codebro");
        std::fs::create_dir_all(&codebro_dir).unwrap();
        let root_str = dir.path().to_string_lossy().to_string();
        let legacy = format!(
            r#"{{
            "schema_version": "1.0.0",
            "workspace_root": "{root_str}",
            "entries": [{{
                "id": "old", "key": "k", "value": "v",
                "metadata": {{"importance": 0.4, "confidence": 0.6, "tags": [], "source": null}},
                "created_at": 100, "last_accessed": 100, "access_count": 0
            }}],
            "updated_at": 100
        }}"#
        );
        std::fs::write(codebro_dir.join("engineering_memory.json"), legacy).unwrap();
        let store = EngineeringMemoryStore::new(dir.path());
        let file: EngineeringMemoryFile = store.load(&root_str).expect("legacy store must load");
        assert_eq!(file.entries.len(), 1);
        assert_eq!(
            file.entries[0].metadata.status,
            crate::engineering_memory::types::MemoryStatus::Active
        );
        assert!(file.entries[0].metadata.expires_at.is_none());
    }
}

/// The result of recording a memory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordOutcome {
    /// Id of the newly stored entry.
    pub id: String,
    /// Conflicts detected and how they were handled.
    pub conflicts: Vec<MemoryConflict>,
}

/// A conflict detected while recording a memory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryConflict {
    pub kind: ConflictKind,
    /// Id of the pre-existing entry involved.
    pub prior_id: String,
    pub prior_key: String,
}

impl MemoryConflict {
    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            ConflictKind::KeyReplaced => "key_replaced",
            ConflictKind::NearDuplicate => "near_duplicate",
        }
    }
}

/// The kind of memory conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Same key existed with a different value; the prior entry is now
    /// `Superseded` and the new entry carries its id in `supersedes`.
    KeyReplaced,
    /// Same key with a highly similar value; both entries remain active.
    NearDuplicate,
}

/// Deterministic token-overlap similarity on whitespace-split lowercase
/// tokens (Jaccard index).
fn token_overlap(a: &str, b: &str) -> f64 {
    let norm = |s: &str| -> std::collections::HashSet<String> {
        s.to_lowercase()
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '.')
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    };
    let sa = norm(a);
    let sb = norm(b);
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

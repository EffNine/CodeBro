//! `EngineeringObjectiveRuntime` — the runtime that loads, persists, and
//! exposes the compact project objective.
//!
//! Mirrors the `ProjectIdentityRuntime` pattern:
//!
//! 1. **Create** — `create()` builds the objective and persists it to
//!    `.codebro/engineering_objective.json`.
//! 2. **Load** — `load()` reads the persisted file, verifying workspace
//!    root and schema, falling back to the documented default objective.
//! 3. **Snapshot** — `snapshot()` returns an immutable `EngineeringObjective`.
//!
//! The runtime never writes on its own during task execution. Persistence is
//! explicit only (`persist()`), matching the engineering memory convention.

use std::path::PathBuf;
use std::time::Instant;

use super::diagnostics::{ObjectiveDiagnostics, ObjectiveSource};
use super::objective::EngineeringObjective;
use super::storage::{ObjectiveStorage, ObjectiveStorageError};

/// Errors that can occur during objective runtime operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectiveRuntimeError {
    Storage(ObjectiveStorageError),
    Generic(String),
}

impl std::fmt::Display for ObjectiveRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectiveRuntimeError::Storage(e) => write!(f, "objective storage: {}", e),
            ObjectiveRuntimeError::Generic(msg) => write!(f, "objective runtime: {}", msg),
        }
    }
}

impl std::error::Error for ObjectiveRuntimeError {}

impl From<ObjectiveStorageError> for ObjectiveRuntimeError {
    fn from(e: ObjectiveStorageError) -> Self {
        ObjectiveRuntimeError::Storage(e)
    }
}

/// Trait for subsystems that provide project objective snapshots.
///
/// Consumers (CanonicalRuntime, Prompt Builder) depend on this trait rather
/// than the concrete runtime, matching the `ProjectIdentityProvider` pattern.
pub trait EngineeringObjectiveProvider {
    /// Returns the provider name for diagnostics.
    fn provider_name(&self) -> &str;

    /// Returns an immutable snapshot of the current objective.
    fn snapshot(&self) -> EngineeringObjective;

    /// Returns diagnostics for the current runtime state.
    fn diagnostics(&self) -> ObjectiveDiagnostics;
}

/// The canonical runtime for the engineering objective.
#[derive(Debug, Clone)]
pub struct EngineeringObjectiveRuntime {
    workspace_root: PathBuf,
    storage: ObjectiveStorage,
    current: EngineeringObjective,
    diagnostics: ObjectiveDiagnostics,
}

impl EngineeringObjectiveRuntime {
    /// Create a new runtime for the given workspace root.
    ///
    /// The objective is not loaded until `load()` or `create()` is called.
    pub fn new(workspace_root: impl AsRef<std::path::Path>) -> Self {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        EngineeringObjectiveRuntime {
            storage: ObjectiveStorage::new(&workspace_root),
            workspace_root,
            current: EngineeringObjective::default(),
            diagnostics: ObjectiveDiagnostics::new(ObjectiveSource::Created),
        }
    }

    /// Workspace root the runtime is scoped to.
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Underlying storage (for tests and diagnostics).
    pub fn storage(&self) -> &ObjectiveStorage {
        &self.storage
    }

    // ── Load ─────────────────────────────────────────────────────────

    /// Load the persisted objective.
    ///
    /// Returns `Ok(false)` when no file exists; the caller may then install
    /// the documented default via [`EngineeringObjectiveRuntime::install_default`]
    /// or `create`. The in-memory objective is never mutated on failure.
    pub fn load(&mut self) -> Result<bool, ObjectiveRuntimeError> {
        let start = Instant::now();
        let root = self.workspace_root.to_string_lossy().to_string();
        match self.storage.load(&root) {
            Ok(file) => {
                self.current = file.objective;
                self.diagnostics = ObjectiveDiagnostics::new(ObjectiveSource::Loaded)
                    .with_load_time(start.elapsed().as_micros() as u64)
                    .with_counts(
                        self.current.success_criteria.len(),
                        self.current.non_goals.len(),
                        !self.current.is_empty(),
                    );
                Ok(true)
            }
            Err(ObjectiveStorageError::NotFound(_)) => {
                self.diagnostics = ObjectiveDiagnostics::new(ObjectiveSource::Created)
                    .with_load_time(start.elapsed().as_micros() as u64);
                Ok(false)
            }
            Err(e) => Err(ObjectiveRuntimeError::Storage(e)),
        }
    }

    // ── Create / Persist ──────────────────────────────────────────────

    /// Persist the given objective to disk.
    pub fn create(&mut self, objective: EngineeringObjective) -> Result<(), ObjectiveRuntimeError> {
        let root = self.workspace_root.to_string_lossy().to_string();
        self.storage.save(&root, &objective)?;
        self.current = objective;
        self.diagnostics = ObjectiveDiagnostics::new(ObjectiveSource::Loaded).with_counts(
            self.current.success_criteria.len(),
            self.current.non_goals.len(),
            !self.current.is_empty(),
        );
        Ok(())
    }

    /// Install a default objective derived from the repository's documented
    /// project goals and persist it. Used when no objective file exists.
    pub fn install_default(
        &mut self,
        objective: EngineeringObjective,
    ) -> Result<(), ObjectiveRuntimeError> {
        let root = self.workspace_root.to_string_lossy().to_string();
        self.storage.save(&root, &objective)?;
        self.current = objective;
        self.diagnostics = ObjectiveDiagnostics::new(ObjectiveSource::Default).with_counts(
            self.current.success_criteria.len(),
            self.current.non_goals.len(),
            !self.current.is_empty(),
        );
        Ok(())
    }

    /// Persist the current in-memory objective to disk (explicit write).
    pub fn persist(&self) -> Result<(), ObjectiveRuntimeError> {
        let root = self.workspace_root.to_string_lossy().to_string();
        self.storage.save(&root, &self.current)?;
        Ok(())
    }

    // ── Snapshot ──────────────────────────────────────────────────────

    /// Return an immutable snapshot of the current objective.
    pub fn snapshot(&self) -> EngineeringObjective {
        self.current.clone()
    }

    /// Reference to the current objective.
    pub fn objective(&self) -> &EngineeringObjective {
        &self.current
    }

    /// Whether an objective file exists on disk.
    pub fn objective_exists(&self) -> bool {
        self.storage.objective_exists()
    }

    /// Return the current diagnostics.
    pub fn diagnostics(&self) -> &ObjectiveDiagnostics {
        &self.diagnostics
    }
}

impl EngineeringObjectiveProvider for EngineeringObjectiveRuntime {
    fn provider_name(&self) -> &str {
        "EngineeringObjectiveRuntime"
    }

    fn snapshot(&self) -> EngineeringObjective {
        self.current.clone()
    }

    fn diagnostics(&self) -> ObjectiveDiagnostics {
        self.diagnostics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn objective() -> EngineeringObjective {
        EngineeringObjective::new("Goal", "Vision", "Objective", "Milestone")
            .with_success_criteria(vec!["c1".to_string()])
            .with_non_goals(vec!["n1".to_string()])
    }

    fn setup() -> (EngineeringObjectiveRuntime, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let runtime = EngineeringObjectiveRuntime::new(tmp.path());
        (runtime, tmp)
    }

    #[test]
    fn test_load_missing_returns_false() {
        let (mut runtime, _tmp) = setup();
        assert!(!runtime.load().expect("load"));
        assert!(runtime.snapshot().is_empty());
    }

    #[test]
    fn test_create_and_reload() {
        let (mut runtime, _tmp) = setup();
        runtime.create(objective()).expect("create");
        assert!(runtime.objective_exists());

        let mut reload = EngineeringObjectiveRuntime::new(_tmp.path());
        assert!(reload.load().expect("reload"));
        assert_eq!(reload.snapshot(), objective());
        assert_eq!(reload.diagnostics().source, ObjectiveSource::Loaded);
    }

    #[test]
    fn test_install_default_persists() {
        let (mut runtime, _tmp) = setup();
        runtime.install_default(objective()).expect("install");
        assert!(runtime.objective_exists());

        let mut reload = EngineeringObjectiveRuntime::new(_tmp.path());
        assert!(reload.load().expect("reload"));
        assert_eq!(reload.snapshot(), objective());
        assert_eq!(reload.diagnostics().source, ObjectiveSource::Loaded);
    }

    #[test]
    fn test_persist_after_mutation() {
        let (mut runtime, tmp) = setup();
        runtime.create(objective()).expect("create");
        // Snapshot is immutable; persist writes the current value.
        runtime.persist().expect("persist");

        let mut reload = EngineeringObjectiveRuntime::new(tmp.path());
        assert!(reload.load().expect("reload"));
        assert_eq!(reload.snapshot().current_objective, "Objective");
    }

    #[test]
    fn test_provider_trait() {
        let (runtime, _tmp) = setup();
        let provider: &dyn EngineeringObjectiveProvider = &runtime;
        assert_eq!(provider.provider_name(), "EngineeringObjectiveRuntime");
        assert!(provider.snapshot().is_empty());
    }

    #[test]
    fn test_deterministic_snapshot() {
        let (mut runtime, _tmp) = setup();
        runtime.create(objective()).expect("create");
        assert_eq!(runtime.snapshot(), runtime.snapshot());
    }
}

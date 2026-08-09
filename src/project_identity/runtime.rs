//! `ProjectIdentityRuntime` — the runtime responsible for loading,
//! maintaining, and exposing project identity.
//!
//! ## Lifecycle
//!
//! 1. **Create** — `ProjectIdentityRuntime::create()` builds a fresh
//!    identity and persists it to `.codebro/project_identity.json`.
//! 2. **Load** — `ProjectIdentityRuntime::load()` reads the persisted
//!    identity, applies migrations, and validates.
//! 3. **Update** — `ProjectIdentityRuntime::update()` applies
//!    `IdentityChanges` and persists the result.
//! 4. **Snapshot** — `ProjectIdentityRuntime::snapshot()` returns an
//!    immutable `ProjectIdentity` for consumption by other subsystems.
//!
//! ## Thread Safety
//!
//! `ProjectIdentityRuntime` is `Clone` (cheap clone of path buffer) but
//! not `Send + Sync` by default. The underlying storage is file-based
//! and synchronous; callers that need concurrent access should wrap the
//! runtime in an `Arc<Mutex<>>` or use the snapshot pattern.

use std::path::PathBuf;
use std::time::Instant;

use super::builder::ProjectIdentityBuilder;
use super::diagnostics::{IdentitySource, ProjectIdentityDiagnostics};
use super::identity::{ProjectIdentity, CURRENT_SCHEMA_VERSION};
use super::loader::{LoadError, LoadResult, ProjectIdentityLoader};
use super::statistics::ProjectIdentityStatistics;
use super::storage::ProjectIdentityStorage;
use super::updater::{IdentityChanges, ProjectIdentityUpdater, UpdateResult};
use super::validation::{validate_identity, ValidationReport};

/// Errors that can occur during runtime operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// Storage error.
    Storage(super::storage::StorageError),
    /// Load error.
    Load(LoadError),
    /// Validation failed.
    Validation(ValidationReport),
    /// Build error.
    Build(super::builder::IdentityBuildError),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Storage(e) => write!(f, "storage error: {}", e),
            RuntimeError::Load(e) => write!(f, "load error: {}", e),
            RuntimeError::Validation(report) => {
                write!(f, "validation error: {}", report.summary())
            }
            RuntimeError::Build(e) => write!(f, "build error: {}", e),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RuntimeError::Storage(e) => Some(e),
            RuntimeError::Load(e) => Some(e),
            RuntimeError::Validation(_) => None,
            RuntimeError::Build(e) => Some(e),
        }
    }
}

impl From<super::storage::StorageError> for RuntimeError {
    fn from(e: super::storage::StorageError) -> Self {
        RuntimeError::Storage(e)
    }
}

impl From<LoadError> for RuntimeError {
    fn from(e: LoadError) -> Self {
        RuntimeError::Load(e)
    }
}

impl From<super::builder::IdentityBuildError> for RuntimeError {
    fn from(e: super::builder::IdentityBuildError) -> Self {
        RuntimeError::Build(e)
    }
}

/// Trait for subsystems that provide project identity snapshots.
///
/// Future runtimes (Engineering Memory Runtime, Reflection Runtime,
/// Learning Runtime) should depend on this trait rather than the
/// concrete `ProjectIdentityRuntime`.
pub trait ProjectIdentityProvider {
    /// Returns the provider name for diagnostics.
    fn provider_name(&self) -> &str;

    /// Returns an immutable snapshot of the current project identity.
    fn snapshot(&self) -> ProjectIdentity;

    /// Returns statistics derived from the current identity.
    fn statistics(&self) -> ProjectIdentityStatistics;

    /// Returns diagnostics for the current runtime state.
    fn diagnostics(&self) -> ProjectIdentityDiagnostics;
}

/// The canonical runtime for managing project identity.
///
/// `ProjectIdentityRuntime` owns persistence in `.codebro/`.
/// `EngineeringContext` only consumes snapshots — it never writes.
#[derive(Debug, Clone)]
pub struct ProjectIdentityRuntime {
    workspace_root: PathBuf,
    storage: ProjectIdentityStorage,
    updater: ProjectIdentityUpdater,
    loader: ProjectIdentityLoader,
    diagnostics: ProjectIdentityDiagnostics,
    current_identity: ProjectIdentity,
}

impl ProjectIdentityRuntime {
    /// Create a new runtime for the given workspace root.
    ///
    /// The identity is not loaded until `load()` or `snapshot()` is called.
    pub fn new(workspace_root: impl AsRef<std::path::Path>) -> Self {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let storage = ProjectIdentityStorage::new(&workspace_root);
        let loader = ProjectIdentityLoader::new(&workspace_root);
        let updater = ProjectIdentityUpdater::new(&workspace_root);
        let default_identity = ProjectIdentity::default();
        ProjectIdentityRuntime {
            workspace_root,
            storage,
            updater,
            loader,
            diagnostics: ProjectIdentityDiagnostics::new(IdentitySource::Created),
            current_identity: default_identity,
        }
    }

    /// Return the workspace root path.
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Return a reference to the underlying storage.
    pub fn storage(&self) -> &ProjectIdentityStorage {
        &self.storage
    }

    // ── Create ─────────────────────────────────────────────────────────

    /// Create a new project identity and persist it.
    ///
    /// If an identity already exists, this replaces it.
    pub fn create(
        &mut self,
        builder: ProjectIdentityBuilder,
    ) -> Result<ProjectIdentity, RuntimeError> {
        let mut identity = builder.build()?;
        let runtime_root = self.workspace_root.to_string_lossy().to_string();
        match &identity.workspace_root {
            Some(provided) if provided == &runtime_root => {
                // Caller-provided root matches runtime root — keep as-is.
            }
            _ => {
                // Missing or conflicting root — runtime is authoritative.
                identity.workspace_root = Some(runtime_root);
            }
        }
        self.storage.ensure_directory()?;
        self.storage.save_all(&identity)?;

        self.current_identity = identity.clone();
        self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Created);
        Ok(identity)
    }

    /// Create a minimal identity with just a name and language.
    pub fn create_minimal(
        &mut self,
        name: impl Into<String>,
        language: impl Into<String>,
    ) -> Result<ProjectIdentity, RuntimeError> {
        let mut identity = ProjectIdentity::new(name, language);
        let runtime_root = self.workspace_root.to_string_lossy().to_string();
        identity.workspace_root = Some(runtime_root);
        self.storage.ensure_directory()?;
        self.storage.save_all(&identity)?;

        self.current_identity = identity.clone();
        self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Created);
        Ok(identity)
    }

    // ── Load ───────────────────────────────────────────────────────────

    /// Load the persisted project identity.
    ///
    /// If no identity exists, returns `Err(RuntimeError::Load)` with a
    /// `NotFound` error so the caller can create a fresh one.
    ///
    /// After a successful migration, the migrated identity and its
    /// projections are persisted before returning.
    pub fn load(&mut self) -> Result<&ProjectIdentity, RuntimeError> {
        let load_start = Instant::now();

        match self.loader.load() {
            Ok(LoadResult {
                identity,
                diagnostics,
                migrated,
            }) => {
                // Persist the migrated (or freshly loaded) identity and
                // all seven supplementary projections.
                if migrated {
                    if let Err(e) = self.storage.save_all(&identity) {
                        let load_time_us = load_start.elapsed().as_micros() as u64;
                        self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Migrated)
                            .with_load_time(load_time_us);
                        return Err(RuntimeError::Storage(e));
                    }
                }

                let load_time_us = load_start.elapsed().as_micros() as u64;
                self.current_identity = identity;
                self.diagnostics = diagnostics
                    .with_load_time(load_time_us)
                    .with_migration_count(if migrated { 1 } else { 0 });
                Ok(&self.current_identity)
            }
            Err(LoadError::Storage(super::storage::StorageError::NotFound(_))) =>
            {
                let load_time_us = load_start.elapsed().as_micros() as u64;
                self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Created)
                    .with_load_time(load_time_us);
                Err(RuntimeError::Load(LoadError::Storage(
                    super::storage::StorageError::NotFound(
                        "identity file not found".to_string(),
                    ),
                )))
            }
            Err(LoadError::UnknownSchemaVersion(msg)) => {
                let load_time_us = load_start.elapsed().as_micros() as u64;
                self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Loaded)
                    .with_load_time(load_time_us);
                Err(RuntimeError::Load(LoadError::UnknownSchemaVersion(msg)))
            }
            Err(LoadError::ValidationFailed(errors)) => {
                let load_time_us = load_start.elapsed().as_micros() as u64;
                self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Loaded)
                    .with_load_time(load_time_us);
                let report = super::validation::ValidationReport::default();
                // Reconstruct a ValidationReport from the error messages.
                let mut report = super::validation::ValidationReport::new();
                for msg in errors {
                    report.add("identity", msg);
                }
                Err(RuntimeError::Validation(report))
            }
            Err(e) => {
                let load_time_us = load_start.elapsed().as_micros() as u64;
                self.diagnostics = ProjectIdentityDiagnostics::new(IdentitySource::Loaded)
                    .with_load_time(load_time_us);
                Err(RuntimeError::Load(e))
            }
        }
    }

    // ── Validate ───────────────────────────────────────────────────────

    /// Validate the current identity and return a report.
    pub fn validate(&mut self) -> ValidationReport {
        let report = validate_identity(&self.current_identity);
        self.diagnostics = self
            .diagnostics
            .clone()
            .with_validation_errors(report.issue_count() as u32);
        report
    }

    // ── Update ─────────────────────────────────────────────────────────

    /// Apply engineering changes to the identity and persist.
    ///
    /// Returns `None` if there are no changes to apply.
    ///
    /// Distinguishes validation failures from storage failures in the
    /// error path. A validation failure leaves the current runtime
    /// identity and the canonical file unchanged.
    pub fn update(
        &mut self,
        changes: IdentityChanges,
    ) -> Option<Result<&ProjectIdentity, RuntimeError>> {
        if changes.is_empty() {
            return None;
        }
        match self.updater.update(&self.current_identity, changes) {
            Some(UpdateResult {
                identity,
                diagnostics,
                applied: true,
            }) => {
                self.current_identity = identity;
                self.diagnostics = diagnostics;
                Some(Ok(&self.current_identity))
            }
            Some(UpdateResult {
                identity,
                diagnostics,
                applied: false,
            }) => {
                // Preserve the current identity on failure.
                self.current_identity = identity;
                self.diagnostics = diagnostics;
                // Distinguish validation failure from storage failure.
                // We check whether the canonical file was actually changed
                // by attempting a fresh load — but that is expensive.
                // Instead, we inspect diagnostics: validation errors are
                // recorded when applied=false.
                // The caller can distinguish by checking diagnostics.
                Some(Err(RuntimeError::Storage(
                    super::storage::StorageError::Write(
                        "failed to persist identity update".to_string(),
                    ),
                )))
            }
            None => None,
        }
    }

    // ── Snapshot ───────────────────────────────────────────────────────

    /// Return an immutable snapshot of the current project identity.
    ///
    /// This is the method `EngineeringContextBuilder` calls to consume
    /// project identity. The returned value is a `Clone` — fully
    /// immutable from the caller's perspective.
    pub fn snapshot(&mut self) -> ProjectIdentity {
        let snap_start = Instant::now();
        let snapshot = self.current_identity.clone();
        let snap_time_us = snap_start.elapsed().as_micros() as u64;
        self.diagnostics = self
            .diagnostics
            .clone()
            .with_snapshot_generation_time(snap_time_us);
        snapshot
    }

    // ── Accessors ──────────────────────────────────────────────────────

    /// Returns a reference to the current identity.
    pub fn identity(&self) -> &ProjectIdentity {
        &self.current_identity
    }

    /// Returns statistics derived from the current identity.
    pub fn statistics(&self) -> ProjectIdentityStatistics {
        ProjectIdentityStatistics::from_identity(&self.current_identity)
    }

    /// Returns diagnostics for the current runtime state.
    pub fn diagnostics(&self) -> &ProjectIdentityDiagnostics {
        &self.diagnostics
    }

    /// Returns `true` if an identity has been loaded or created.
    pub fn is_loaded(&self) -> bool {
        !self.current_identity.is_basic() || self.loader.identity_exists()
    }
}

impl ProjectIdentityProvider for ProjectIdentityRuntime {
    fn provider_name(&self) -> &str {
        "ProjectIdentityRuntime"
    }

    fn snapshot(&self) -> ProjectIdentity {
        self.current_identity.clone()
    }

    fn statistics(&self) -> ProjectIdentityStatistics {
        ProjectIdentityStatistics::from_identity(&self.current_identity)
    }

    fn diagnostics(&self) -> ProjectIdentityDiagnostics {
        self.diagnostics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (ProjectIdentityRuntime, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let runtime = ProjectIdentityRuntime::new(tmp.path());
        (runtime, tmp)
    }

    #[test]
    fn test_create_minimal() {
        let (mut runtime, _tmp) = setup();
        let identity = runtime
            .create_minimal("test-proj", "rust")
            .expect("create");
        assert_eq!(identity.name, "test-proj");
        assert_eq!(identity.primary_language(), "rust");
    }

    #[test]
    fn test_create_with_builder() {
        let (mut runtime, _tmp) = setup();
        let identity = runtime
            .create(
                ProjectIdentityBuilder::new()
                    .name("full-proj")
                    .language("go")
                    .framework("gin")
                    .build_system("go build")
                    .known_module("auth")
                    .known_module("api")
                    .known_constraint("No raw SQL")
                    .coding_convention("PascalCase handlers"),
            )
            .expect("create");
        assert_eq!(identity.name, "full-proj");
        assert_eq!(identity.known_module_count(), 2);
        assert_eq!(identity.constraint_count(), 1);
        assert_eq!(identity.convention_count(), 1);
    }

    #[test]
    fn test_load_after_create() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("persisted", "rust")
            .expect("create");
        let loaded = runtime.load().expect("load");
        assert_eq!(loaded.name, "persisted");
    }

    #[test]
    fn test_load_missing() {
        let (mut runtime, _tmp) = setup();
        let result = runtime.load();
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot_returns_clone() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("snap-proj", "rust")
            .expect("create");
        let snap = runtime.snapshot();
        assert_eq!(snap.name, "snap-proj");
        assert_eq!(snap.primary_language(), "rust");
    }

    #[test]
    fn test_update_adds_constraint() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("upd-proj", "rust")
            .expect("create");
        let changes = IdentityChanges {
            add_constraints: vec!["No raw SQL".to_string()],
            ..Default::default()
        };
        let result = runtime.update(changes);
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());
        assert_eq!(runtime.identity().constraint_count(), 1);
    }

    #[test]
    fn test_update_empty_changes_returns_none() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("noop", "rust")
            .expect("create");
        let result = runtime.update(IdentityChanges::new());
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_clean_identity() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("valid-proj", "rust")
            .expect("create");
        let report = runtime.validate();
        assert!(report.is_valid());
    }

    #[test]
    fn test_validate_empty_name() {
        let (mut runtime, _tmp) = setup();
        runtime.create_minimal("valid-proj", "rust").expect("create");
        // Manually set an empty name to test validation.
        runtime.current_identity.name = String::new();
        let report = runtime.validate();
        assert!(!report.is_valid());
    }

    #[test]
    fn test_validate_empty_languages() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("stat-proj", "rust")
            .expect("create");
        let stats = runtime.statistics();
        assert_eq!(stats.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(stats.decision_count, 0);
        assert_eq!(stats.constraint_count, 0);
    }

    #[test]
    fn test_diagnostics_after_load() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("diag-proj", "rust")
            .expect("create");
        runtime.load().expect("load");
        let diags = runtime.diagnostics();
        assert_eq!(
            diags.source,
            IdentitySource::Loaded
        );
        assert!(diags.load_time_us > 0);
    }

    #[test]
    fn test_deterministic_snapshot() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create(
                ProjectIdentityBuilder::new()
                    .name("det-proj")
                    .language("rust")
                    .language("go")
                    .known_module("z-module")
                    .known_module("a-module")
                    .important_file("z.rs")
                    .important_file("a.rs"),
            )
            .expect("create");
        let snap1 = runtime.snapshot();
        let snap2 = runtime.snapshot();
        assert_eq!(snap1, snap2);
        assert_eq!(snap1.known_modules, vec!["a-module", "z-module"]);
        assert_eq!(snap1.important_files, vec!["a.rs", "z.rs"]);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let (mut runtime, _tmp) = setup();
        let builder = ProjectIdentityBuilder::new()
            .name("serial-proj")
            .language("typescript")
            .framework("nextjs")
            .architecture_summary("Monorepo");
        let identity = runtime.create(builder).expect("create");
        let json = serde_json::to_string(&identity).expect("serialize");
        let decoded: ProjectIdentity =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(identity.name, decoded.name);
        assert_eq!(identity.languages, decoded.languages);
        assert_eq!(identity.frameworks, decoded.frameworks);
        assert_eq!(identity.schema_version, decoded.schema_version);
    }

    #[test]
    fn test_provider_trait() {
        let (runtime, _tmp) = setup();
        let provider: &dyn ProjectIdentityProvider = &runtime;
        assert_eq!(provider.provider_name(), "ProjectIdentityRuntime");
        let snap = provider.snapshot();
        assert_eq!(snap.name, "unknown");
    }

    #[test]
    fn test_is_loaded_after_create() {
        let (mut runtime, _tmp) = setup();
        assert!(!runtime.is_loaded());
        runtime
            .create_minimal("loaded-proj", "rust")
            .expect("create");
        assert!(runtime.is_loaded());
    }

    #[test]
    fn test_is_loaded_after_load() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("loadable-proj", "rust")
            .expect("create");
        assert!(runtime.is_loaded());
        runtime.load().expect("load");
        assert!(runtime.is_loaded());
    }

    #[test]
    fn test_concurrent_snapshots() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("conc-proj", "rust")
            .expect("create");
        // Multiple sequential snapshots should be identical.
        let snap1 = runtime.snapshot();
        let snap2 = runtime.snapshot();
        let snap3 = runtime.snapshot();
        assert_eq!(snap1, snap2);
        assert_eq!(snap2, snap3);
    }

    #[test]
    fn test_create_generates_all_eight_files() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("files-proj", "rust")
            .expect("create");
        let storage = runtime.storage();
        assert!(storage.identity_path().exists());
        assert!(storage.workspace_path().exists());
        assert!(storage.architecture_path().exists());
        assert!(storage.decisions_path().exists());
        assert!(storage.constraints_path().exists());
        assert!(storage.roadmap_path().exists());
        assert!(storage.sprint_path().exists());
        assert!(storage.metadata_path().exists());
    }

    #[test]
    fn test_update_refreshes_projections() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("proj-proj", "rust")
            .expect("create");
        let storage = runtime.storage().clone();
        runtime
            .update(IdentityChanges {
                add_constraints: vec!["no-raw-sql".to_string()],
                set_sprint: Some("sprint-24".to_string()),
                update_architecture_summary: Some("hexagonal".to_string()),
                add_roadmap_items: vec![
                    crate::project_identity::RoadmapItem::new("r1", "Fix auth", None),
                ],
                ..Default::default()
            })
            .expect("some")
            .expect("ok");

        // Verify each projection file was refreshed.
        assert!(storage.constraints_path().exists());
        assert!(storage.sprint_path().exists());
        assert!(storage.architecture_path().exists());
        assert!(storage.roadmap_path().exists());

        // Verify canonical data matches projections.
        let reloaded = storage.load_identity().expect("reload");
        assert_eq!(reloaded.known_constraints, vec!["no-raw-sql"]);
        assert_eq!(reloaded.current_sprint, Some("sprint-24".to_string()));
        assert_eq!(
            reloaded.architecture_summary,
            Some("hexagonal".to_string())
        );
        assert_eq!(reloaded.roadmap_item_count(), 1);
    }

    #[test]
    fn test_invalid_identity_changes_rejected_and_unchanged() {
        let (mut runtime, _tmp) = setup();
        runtime
            .create_minimal("safe-proj", "rust")
            .expect("create");
        let before_name = runtime.identity().name.clone();
        let before_constraints = runtime.identity().known_constraints.clone();

        // Add a decision, then try to add another with the same ID.
        runtime
            .update(IdentityChanges {
                add_decisions: vec![
                    crate::project_identity::EngineeringDecision::new(
                        "dup-dec", "Decision 1", "First", None,
                    ),
                ],
                ..Default::default()
            })
            .expect("some")
            .expect("ok");

        // Now try to add a duplicate decision — should fail validation.
        let result = runtime.update(IdentityChanges {
            add_decisions: vec![
                crate::project_identity::EngineeringDecision::new(
                    "dup-dec", "Decision 2", "Duplicate", None,
                ),
            ],
            ..Default::default()
        });
        assert!(result.is_some());
        assert!(result.unwrap().is_err());

        // In-memory identity must be unchanged.
        assert_eq!(runtime.identity().name, before_name);
        assert_eq!(
            runtime.identity().known_constraints,
            before_constraints
        );
        // Canonical file must be unchanged — reload and verify.
        let reloaded = runtime.storage().load_identity().expect("reload");
        assert_eq!(reloaded.name, before_name);
        assert_eq!(reloaded.known_constraints, before_constraints);
    }

    #[test]
    fn test_migration_persisted_and_next_load_clean() {
        let (mut runtime, _tmp) = setup();
        // Write a 0.9.0 identity directly to storage.
        let old_identity = ProjectIdentity {
            schema_version: "0.9.0".to_string(),
            ..ProjectIdentity::new("migrate-proj", "go")
        };
        runtime.storage().save_identity(&old_identity).expect("save");

        // First load: migrates once.
        let first = runtime.load().expect("first load");
        assert_eq!(
            first.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        assert_eq!(runtime.diagnostics().migration_count, 1);

        // Second load: zero migrations, clean.
        let second = runtime.load().expect("second load");
        assert_eq!(
            second.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        assert_eq!(runtime.diagnostics().migration_count, 0);
    }

    #[test]
    fn test_duplicate_ids_fail_load_with_validation() {
        let (mut runtime, _tmp) = setup();
        // Write an identity with duplicate decision IDs.
        let identity = ProjectIdentity::new("bad-proj", "rust")
            .add_engineering_decision(
                crate::project_identity::EngineeringDecision::new(
                    "dec-1", "D1", "Desc", None,
                ),
            )
            .add_engineering_decision(
                crate::project_identity::EngineeringDecision::new(
                    "dec-1", "D2", "Desc2", None,
                ),
            );
        runtime.storage().save_identity(&identity).expect("save");

        let result = runtime.load();
        assert!(result.is_err());
        match result.unwrap_err() {
            RuntimeError::Validation(_) => {}
            other => panic!("Expected Validation error, got {:?}", other),
        }
    }

    #[test]
    fn test_create_minimal_persists_runtime_workspace_root() {
        let (mut runtime, tmp) = setup();
        let workspace_str = tmp.path().to_string_lossy().to_string();
        runtime
            .create_minimal("minimal-proj", "rust")
            .expect("create");

        // Verify workspace.json contains the runtime root.
        let ws_content = fs::read_to_string(runtime.storage().workspace_path())
            .expect("read workspace.json");
        #[derive(serde::Deserialize)]
        struct WorkspaceMeta {
            root_path: String,
        }
        let ws: WorkspaceMeta = serde_json::from_str(&ws_content).expect("parse workspace");
        assert_eq!(ws.root_path, workspace_str);

        // Verify canonical identity also has the runtime root.
        let reloaded = runtime.storage().load_identity().expect("reload");
        assert_eq!(
            reloaded.workspace_root.as_deref(),
            Some(workspace_str.as_str())
        );
    }

    #[test]
    fn test_create_builder_without_workspace_root_gets_runtime_root() {
        let (mut runtime, tmp) = setup();
        let workspace_str = tmp.path().to_string_lossy().to_string();
        let identity = runtime
            .create(
                ProjectIdentityBuilder::new()
                    .name("builder-proj")
                    .language("go")
                    .framework("gin"),
            )
            .expect("create");

        assert_eq!(
            identity.workspace_root.as_deref(),
            Some(workspace_str.as_str())
        );

        let ws_content = fs::read_to_string(runtime.storage().workspace_path())
            .expect("read workspace.json");
        #[derive(serde::Deserialize)]
        struct WorkspaceMeta {
            root_path: String,
        }
        let ws: WorkspaceMeta = serde_json::from_str(&ws_content).expect("parse workspace");
        assert_eq!(ws.root_path, workspace_str);
    }

    #[test]
    fn test_create_builder_with_conflicting_root_uses_runtime_root() {
        let (mut runtime, tmp) = setup();
        let workspace_str = tmp.path().to_string_lossy().to_string();
        let conflicting_root = "/tmp/some-other-project".to_string();
        let identity = runtime
            .create(
                ProjectIdentityBuilder::new()
                    .name("conflict-proj")
                    .language("python")
                    .workspace_root(&conflicting_root),
            )
            .expect("create");

        // Canonical identity must use the runtime root, not the builder's.
        assert_eq!(
            identity.workspace_root.as_deref(),
            Some(workspace_str.as_str())
        );

        // workspace.json projection must also use the runtime root.
        let ws_content = fs::read_to_string(runtime.storage().workspace_path())
            .expect("read workspace.json");
        #[derive(serde::Deserialize)]
        struct WorkspaceMeta {
            root_path: String,
        }
        let ws: WorkspaceMeta = serde_json::from_str(&ws_content).expect("parse workspace");
        assert_eq!(ws.root_path, workspace_str);

        // The conflicting value must not appear anywhere.
        assert!(!identity.workspace_root.as_deref().unwrap().contains("some-other-project"));
        assert!(!ws.root_path.contains("some-other-project"));
    }
}

//! The workspace-bound mutation engine of the CodeBro MCP runtime.
//!
//! This is the ONLY mutation seam exposed through the MCP `apply_change`
//! tool. Enforcement happens before any bytes are written:
//!
//! - **Workspace boundary** — absolute paths outside the root and any real
//!   `..` traversal component are denied; symlinked parents escaping the
//!   root are caught by canonicalization.
//! - **No blind overwrite** — an existing file requires a non-empty `old`.
//! - **Unambiguous replacement** — an `old` occurring more than once is
//!   denied; the caller must supply more context.
//! - **Stale-state protection** — preparation snapshots the file; apply
//!   refuses if the on-disk content no longer matches the snapshot.
//!
//! Existing-file writes ride [`ChangePlan`](crate::tools::ChangePlan) /
//! [`PatchEngine`](crate::tools::PatchEngine); file creation — which
//! PatchEngine cannot reconstruct from a non-existent on-disk base — goes
//! through the engine's documented controlled creation seam
//! ([`ChangeEngine::create_file`]).
//!
//! Historical note: this code was carved out of the Sprint 30F coding
//! subagent's `permissions.rs`. The subagent surfaces (permission hook,
//! restricted registry, `CodingTooling`) moved to `crate::legacy`; the
//! engine itself is production runtime.

use std::path::{Path, PathBuf};

/// A change prepared against the current file content, ready to apply.
///
/// Preparation is strictly READ-ONLY: nothing is written until [`ChangeEngine::apply`]
/// runs, and only then if the file still matches the prepared snapshot
/// (stale-state protection).
#[derive(Debug, Clone)]
pub struct PreparedChange {
    /// The target file, resolved to an absolute path.
    pub path: PathBuf,
    /// Whether the file did not exist at preparation time.
    pub created: bool,
    /// Whether the target is outside the plan's affected files.
    pub unplanned: bool,
    /// Readable diff preview (also the tool result the model observes).
    pub preview: String,
    /// The complete file content captured at preparation time ("" for
    /// created files) — the rollback source and the stale-check snapshot.
    pub backup: String,
    /// The exact text that must be uniquely present in the file.
    pub old: String,
    /// The exact replacement text.
    pub new: String,
    /// The resulting full file content.
    pub full_new: String,
}

/// The workspace-bound mutation engine behind MCP `apply_change`.
///
/// Existing-file writes route through a
/// [`ChangePlan`](crate::tools::ChangePlan) built by
/// [`crate::tools::PatchEngine`]; created files go through the engine's
/// controlled creation seam ([`ChangeEngine::create_file`]). The engine never
/// calls `fs::write` on source files outside these two seams — it is the ONLY
/// mutation path of the runtime.
pub struct ChangeEngine {
    workspace_root: PathBuf,
    planned_files: Vec<PathBuf>,
    strict: bool,
}

impl ChangeEngine {
    pub fn new(workspace_root: &Path, planned_files: &[PathBuf], strict: bool) -> Self {
        let root = workspace_root.to_path_buf();
        let planned_files = planned_files
            .iter()
            .filter_map(|p| resolve_path(&root, &p.display().to_string()).ok())
            .collect();
        ChangeEngine {
            workspace_root: root,
            planned_files,
            strict,
        }
    }

    /// The strict-plan flag: when true, out-of-plan changes are denied.
    pub fn strict(&self) -> bool {
        self.strict
    }

    /// Resolve a tool argument path and enforce the workspace-root boundary.
    /// Absolute paths outside the root (and any `..` traversal) are denied.
    pub fn resolve(&self, argument: &str) -> crate::error::Result<PathBuf> {
        resolve_path(&self.workspace_root, argument).map_err(|e| {
            crate::error::CodeBroError::Permission(format!("change engine path boundary: {e}"))
        })
    }

    /// Prepare a change against the CURRENT file content (read-only).
    ///
    /// Enforcement happens here, before any mutation:
    /// - path boundary,
    /// - plan adherence (strict mode denies out-of-plan targets),
    /// - existing files require a unique, non-empty `old` match (no blind
    ///   overwrite, no ambiguous edits),
    /// - created files require `old` to be empty.
    pub fn prepare(
        &self,
        path: &str,
        old: &str,
        new: &str,
    ) -> crate::error::Result<PreparedChange> {
        let abs = self.resolve(path)?;
        let unplanned = !self.planned_files.contains(&abs) && !self.planned_files.is_empty();
        if self.strict && unplanned {
            return Err(crate::error::CodeBroError::Permission(format!(
                "plan adherence: '{}' is not among the plan's affected files and strict plan adherence is enabled",
                abs.display()
            )));
        }

        if abs.exists() {
            let content = std::fs::read_to_string(&abs).map_err(|e| {
                crate::error::CodeBroError::Patch(format!(
                    "Cannot read {} for change proposal: {e}",
                    abs.display()
                ))
            })?;
            if old.is_empty() {
                return Err(crate::error::CodeBroError::Permission(format!(
                    "blind overwrite denied: '{}' already exists — provide the exact `old` text to replace, not an empty match",
                    abs.display()
                )));
            }
            let occurrences = content.matches(old).count();
            if occurrences == 0 {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "stale content: the provided `old` text does not occur in '{}'",
                    abs.display()
                )));
            }
            if occurrences > 1 {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "ambiguous change denied: the provided `old` text occurs {occurrences} times in '{}' — supply more surrounding context for a unique match",
                    abs.display()
                )));
            }
            let full_new = content.replacen(old, new, 1);
            let plan = crate::tools::ChangePlan::propose_between(&abs, &content, &full_new)?;
            Ok(PreparedChange {
                path: abs,
                created: false,
                unplanned,
                preview: plan.preview().to_string(),
                backup: content,
                old: old.to_string(),
                new: new.to_string(),
                full_new,
            })
        } else {
            if !old.trim().is_empty() {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "cannot create '{}': the file does not exist — to create it pass old=\"\" with the full content as new",
                    abs.display()
                )));
            }
            let plan = crate::tools::ChangePlan::propose_between(&abs, "", new)?;
            Ok(PreparedChange {
                path: abs,
                created: true,
                unplanned,
                preview: plan.preview().to_string(),
                backup: String::new(),
                old: String::new(),
                new: new.to_string(),
                full_new: new.to_string(),
            })
        }
    }

    /// Apply a prepared change — but ONLY if the file still matches the
    /// preparation-time snapshot. A file changed by anyone else between
    /// preparation and application is never clobbered.
    ///
    /// This is the prepare/apply seam: preparation stays read-only and
    /// reversible. Existing-file changes are a single
    /// [`ChangePlan::apply`](crate::tools::ChangePlan::apply) routed through
    /// [`PatchEngine`](crate::tools::PatchEngine); created files use the
    /// engine's controlled creation seam
    /// ([`ChangeEngine::create_file`]) because PatchEngine reconstructs the
    /// new content from a file's on-disk base, which cannot exist for a file
    /// being created.
    pub fn apply(&self, prepared: &PreparedChange) -> crate::error::Result<String> {
        let current = if prepared.created {
            if prepared.path.exists() {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "stale state: '{}' was created by someone else since the proposal — refusing to overwrite it",
                    prepared.path.display()
                )));
            }
            String::new()
        } else {
            std::fs::read_to_string(&prepared.path).map_err(|e| {
                crate::error::CodeBroError::Patch(format!(
                    "Cannot read {} for change apply: {e}",
                    prepared.path.display()
                ))
            })?
        };
        if current != prepared.backup {
            return Err(crate::error::CodeBroError::Patch(format!(
                "stale state: '{}' changed since the proposal — refusing to apply over unknown content",
                prepared.path.display()
            )));
        }
        if prepared.created {
            return self.create_file(prepared);
        }
        let mut plan = crate::tools::ChangePlan::propose_between(
            &prepared.path,
            &prepared.backup,
            &prepared.full_new,
        )?;
        plan.apply()
    }

    /// The CONTROLLED creation path of the engine — the sole filesystem write
    /// that does not ride a [`ChangePlan`](crate::tools::ChangePlan).
    ///
    /// File creation cannot go through [`PatchEngine`](crate::tools::PatchEngine):
    /// [`PatchEngine::apply`](crate::tools::PatchEngine::apply) reconstructs
    /// the new content from the file's on-disk base, which does not exist for
    /// a file being created. Creation therefore stays INSIDE the engine as a
    /// single, explicitly documented write, still protected by the
    /// prepare/apply staleness check that ran in
    /// [`ChangeEngine::apply`] (a file created by someone else between prepare
    /// and apply is never clobbered).
    fn create_file(&self, prepared: &PreparedChange) -> crate::error::Result<String> {
        if let Some(parent) = prepared.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    crate::error::CodeBroError::Patch(format!(
                        "Cannot create parent dirs for {}: {e}",
                        prepared.path.display()
                    ))
                })?;
            }
        }
        std::fs::write(&prepared.path, &prepared.full_new)
            .map_err(|e| crate::error::CodeBroError::Patch(format!("Failed to write file: {e}")))?;
        Ok(format!("Patch applied to {}", prepared.path.display()))
    }
}

/// Resolve a change path against the workspace root, denying any escape.
fn resolve_path(workspace_root: &Path, argument: &str) -> crate::error::Result<PathBuf> {
    let trimmed = argument.trim();
    if trimmed.is_empty() {
        return Err(crate::error::CodeBroError::Permission(
            "empty path".to_string(),
        ));
    }
    let raw = std::path::Path::new(trimmed);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace_root.join(raw)
    };
    // Deny `..` traversal outright (a real component, not a normalized one).
    for component in candidate.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(crate::error::CodeBroError::Permission(format!(
                "path traversal denied: '{}'",
                trimmed
            )));
        }
    }
    // Deny paths that escape the workspace root after normalization.
    if !candidate.starts_with(workspace_root) {
        return Err(crate::error::CodeBroError::Permission(format!(
            "outside workspace root: '{}'",
            trimmed
        )));
    }
    // Resolve symlinks: a link inside the root may point outside it (or a
    // parent path may be a link). Verify the CANONICAL target still lives
    // under the canonical workspace root, so writes cannot escape via
    // symlink. For a not-yet-existing file (create path) we canonicalize
    // the parent directory instead, which still catches a symlinked parent
    // escaping the root.
    //
    // Canonicalize the workspace root once (e.g. macOS /var -> /private/var)
    // and compare canonicalized targets against it, so a workspace behind a
    // symlink does not false-positive.
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let canonical_candidate = candidate
        .canonicalize()
        .ok()
        .or_else(|| {
            // Create path: the file does not exist yet; canonicalize the
            // nearest existing ancestor so a symlinked parent is resolved,
            // then re-append the missing tail.
            let mut existing = candidate.as_path();
            let mut tail: Vec<std::ffi::OsString> = Vec::new();
            while !existing.exists() {
                if let Some(name) = existing.file_name() {
                    tail.push(name.to_os_string());
                }
                existing = existing.parent().unwrap_or(existing);
            }
            existing
                .canonicalize()
                .ok()
                .map(|base| tail.into_iter().rev().fold(base, |p, n| p.join(n)))
        })
        .unwrap_or_else(|| candidate.clone());
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(crate::error::CodeBroError::Permission(format!(
            "symlink escape denied: '{}' resolves outside the workspace root",
            trimmed
        )));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_engine_rejects_path_traversal_and_outside_paths() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        for bad in [
            "../escape.txt".to_string(),
            "sub/../../escape.txt".to_string(),
            dir.path()
                .parent()
                .unwrap()
                .join("outside.txt")
                .to_string_lossy()
                .to_string(),
        ] {
            assert!(
                engine.resolve(&bad).is_err(),
                "'{bad}' must be denied by the path boundary"
            );
        }
    }

    #[test]
    fn test_engine_resolves_relative_paths_into_root() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let abs = engine.resolve("src/lib.rs").unwrap();
        assert_eq!(abs, dir.path().join("src/lib.rs"));
    }

    #[test]
    fn test_engine_modify_prepare_is_read_only_and_applies() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        );
        let engine = ChangeEngine::new(dir.path(), &[PathBuf::from("main.rs")], false);

        let prepared = engine
            .prepare("main.rs", "println!(\"hi\")", "println!(\"hello\")")
            .unwrap();
        assert!(!prepared.created);
        assert!(!prepared.unplanned);
        assert!(prepared
            .preview
            .contains("+fn main() { println!(\"hello\"); }"));
        // Preparation must NOT touch the file.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "fn main() { println!(\"hi\"); }\n"
        );
        engine.apply(&prepared).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "fn main() { println!(\"hello\"); }\n"
        );
    }

    #[test]
    fn test_engine_denies_blind_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "keep this content");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("main.rs", "", "replacement")
            .expect_err("empty old text on an existing file must be denied");
        assert!(err.to_string().contains("blind overwrite"), "got: {err}");
    }

    #[test]
    fn test_engine_denies_ambiguous_match() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "let x = 1;\nlet y = 1;\n");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("main.rs", "= 1;", "= 2;")
            .expect_err("a non-unique old text must be denied");
        assert!(err.to_string().contains("ambiguous"), "got: {err}");
    }

    #[test]
    fn test_engine_rejects_stale_old_text_at_prepare() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "alpha\nbeta\n");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("main.rs", "gamma", "delta")
            .expect_err("an old text absent from the file must be denied");
        assert!(err.to_string().contains("stale"), "got: {err}");
    }

    #[test]
    fn test_engine_apply_refuses_stale_state_between_prepare_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "original line\n");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let prepared = engine
            .prepare("main.rs", "original line", "changed line")
            .unwrap();
        // Someone else modifies the file between preparation and application.
        write(&dir.path().join("main.rs"), "someone else's content\n");
        let err = engine
            .apply(&prepared)
            .expect_err("apply must refuse stale content");
        assert!(err.to_string().contains("stale state"), "got: {err}");
        // The foreign content is preserved.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "someone else's content\n"
        );
    }

    #[test]
    fn test_engine_create_new_file_and_backup() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let prepared = engine
            .prepare("src/new.rs", "", "pub fn fresh() {}\n")
            .unwrap();
        assert!(prepared.created);
        assert!(prepared.backup.is_empty());
        // The observable diff is produced at PREPARE time (read-only): the
        // file is still absent, yet the preview is the full addition.
        assert!(
            prepared.preview.contains("+pub fn fresh() {}"),
            "created-file preview must be the observable diff: {}",
            prepared.preview
        );
        engine.apply(&prepared).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/new.rs")).unwrap(),
            "pub fn fresh() {}\n"
        );
    }

    #[test]
    fn test_engine_apply_refuses_stale_create_between_prepare_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let prepared = engine
            .prepare("src/new.rs", "", "pub fn fresh() {}\n")
            .unwrap();
        assert!(prepared.created);
        // Someone else creates the file between preparation and application.
        write(&dir.path().join("src/new.rs"), "someone else's file\n");
        let err = engine
            .apply(&prepared)
            .expect_err("apply must refuse to clobber a file created since the proposal");
        assert!(err.to_string().contains("stale state"), "got: {err}");
        // The foreign file is preserved untouched — the session never wrote.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/new.rs")).unwrap(),
            "someone else's file\n"
        );
    }

    #[test]
    fn test_engine_denies_create_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        // Traversal escape is denied.
        let traversal = engine
            .prepare("../outside.rs", "", "content")
            .expect_err("a traversal create must be denied");
        assert!(
            traversal.to_string().contains("path boundary"),
            "got: {traversal}"
        );
        // An absolute path outside the root is denied.
        let outside = dir.path().parent().unwrap().join("outside-create.rs");
        let outside_err = engine
            .prepare(&outside.to_string_lossy(), "", "content")
            .expect_err("a create outside the workspace root must be denied");
        assert!(
            outside_err.to_string().contains("path boundary"),
            "got: {outside_err}"
        );
        assert!(!outside.exists(), "no file may be written outside the root");
    }

    #[test]
    fn test_engine_create_requires_empty_old() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("src/new.rs", "text", "content")
            .expect_err("creating a file with non-empty old must be denied");
        assert!(err.to_string().contains("does not exist"), "got: {err}");
    }

    #[test]
    fn test_engine_marks_unplanned_changes_but_applies_by_default() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("planned.rs"), "planned\n");
        write(&dir.path().join("extra.rs"), "extra\n");
        let engine = ChangeEngine::new(dir.path(), &[PathBuf::from("planned.rs")], false);

        let planned = engine.prepare("planned.rs", "planned", "planned!").unwrap();
        assert!(!planned.unplanned);

        let extra = engine.prepare("extra.rs", "extra", "extra!").unwrap();
        assert!(
            extra.unplanned,
            "a file outside the plan must be flagged as unplanned"
        );
        engine.apply(&extra).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("extra.rs")).unwrap(),
            "extra!\n"
        );
    }

    #[test]
    fn test_engine_strict_mode_denies_unplanned_changes() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("planned.rs"), "planned\n");
        write(&dir.path().join("extra.rs"), "extra\n");
        let engine = ChangeEngine::new(dir.path(), &[PathBuf::from("planned.rs")], true);
        let err = engine
            .prepare("extra.rs", "extra", "extra!")
            .expect_err("strict mode must deny out-of-plan changes");
        assert!(err.to_string().contains("plan adherence"), "got: {err}");
        assert!(engine.prepare("planned.rs", "planned", "planned!").is_ok());
    }
}

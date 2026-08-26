#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use std::path::PathBuf;

use crate::error::CodeBroError;
use crate::tools::patch::{FilePatch, PatchEngine};

/// A single, reviewable change to one file.
///
/// Guarantees of the code-change workflow (Task 4):
///   - Files are NEVER modified until `apply()` runs on an explicitly
///     approved plan.
///   - Every plan carries the original bytes for rollback.
pub struct ChangePlan {
    pub patch: FilePatch,
    backup_original: String,
    applied: bool,
}

impl ChangePlan {
    /// Research + generate a patch for `file`, replacing its current content
    /// with `new_content`. This is read-only: nothing is written.
    pub fn propose(file: &PathBuf, new_content: &str) -> crate::error::Result<Self> {
        let old_content = std::fs::read_to_string(file).map_err(|e| {
            CodeBroError::Patch(format!(
                "Cannot read {} for change proposal: {e}",
                file.display()
            ))
        })?;
        let patch = PatchEngine::create_patch(file, &old_content, new_content)?;
        Ok(ChangePlan {
            patch,
            backup_original: old_content,
            applied: false,
        })
    }

    /// Build a change plan from explicit old/new content (used when the target
    /// snapshot is known ahead of time, e.g. for freshly-created files).
    pub fn propose_between(
        file: &PathBuf,
        old_content: &str,
        new_content: &str,
    ) -> crate::error::Result<Self> {
        let patch = PatchEngine::create_patch(file, old_content, new_content)?;
        Ok(ChangePlan {
            patch,
            backup_original: old_content.to_string(),
            applied: false,
        })
    }

    pub fn preview(&self) -> &str {
        PatchEngine::preview(&self.patch)
    }

    pub fn path(&self) -> &PathBuf {
        &self.patch.path
    }

    /// Apply the change. Intended to run only after explicit user approval.
    pub fn apply(&mut self) -> crate::error::Result<String> {
        if self.applied {
            return Ok("Change already applied (no-op).".to_string());
        }
        PatchEngine::validate_patch(&self.patch)?;
        let out = PatchEngine::apply(&self.patch, false)?;
        self.applied = true;
        Ok(out)
    }

    /// Restore the original bytes captured when the plan was created.
    pub fn rollback(&self) -> crate::error::Result<()> {
        PatchEngine::rollback(&self.patch.path, &self.backup_original)
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_propose_is_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "fn main() { println!(\"hello\"); }\n").unwrap();

        let plan = ChangePlan::propose(&path, "fn main() { println!(\"world\"); }\n").unwrap();
        // Proposal must not touch the file.
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "fn main() { println!(\"hello\"); }\n"
        );
        assert!(plan.preview().contains("-fn main()"));
        assert!(plan.preview().contains("+fn main()"));
        assert!(!plan.is_applied());
    }

    #[test]
    fn test_apply_then_rollback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "old\n").unwrap();

        let mut plan = ChangePlan::propose(&path, "new\n").unwrap();
        plan.apply().expect("apply should succeed");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert!(plan.is_applied());

        plan.rollback().expect("rollback should succeed");
        assert_eq!(fs::read_to_string(&path).unwrap(), "old\n");
    }

    #[test]
    fn test_apply_requires_explicit_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "abc\n").unwrap();

        let plan = ChangePlan::propose(&path, "def\n").unwrap();
        // Merely building a plan must not modify the file.
        assert_eq!(fs::read_to_string(&path).unwrap(), "abc\n");
        drop(plan);
        assert_eq!(fs::read_to_string(&path).unwrap(), "abc\n");
    }

    #[test]
    fn test_propose_between_for_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("new.txt");
        let plan = ChangePlan::propose_between(&path, "", "hello\n").unwrap();
        assert!(plan.preview().contains("+hello"));
    }
}

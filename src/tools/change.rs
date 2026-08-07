#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use std::path::PathBuf;
use std::process::Command;

use crate::error::CodeBroError;
use crate::tools::patch::{FilePatch, PatchEngine, PatchSet};

/// A single, reviewable change to one file.
///
/// Guarantees of the code-change workflow (Task 4):
///   - Files are NEVER modified until `apply()` runs on an explicitly
///     approved plan.
///   - Every plan carries the original bytes for rollback.
///   - `apply_and_verify` rolls back automatically if a post-apply
///     verification command fails.
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

    /// Apply, then run `verify_cmd`. If verification fails, roll back and
    /// return an error. Pass `None` to apply without a verification gate.
    pub fn apply_and_verify(&mut self, verify_cmd: Option<&str>) -> crate::error::Result<String> {
        let applied = self.apply()?;
        if let Some(cmd) = verify_cmd {
            let status = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .status()
                .map_err(|e| {
                    CodeBroError::Patch(format!("Verification command failed to run: {e}"))
                })?;
            if !status.success() {
                let code = status.code();
                let _ = self.rollback();
                return Err(CodeBroError::Patch(format!(
                    "Post-apply verification failed (exit {code:?}); change rolled back."
                )));
            }
        }
        Ok(applied)
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }
}

/// Collects a set of pending, reviewable changes for a single approval pass.
pub struct ChangeReview {
    pub patches: PatchSet,
    pub backups: Vec<(PathBuf, String)>,
}

impl ChangeReview {
    pub fn new() -> Self {
        ChangeReview {
            patches: PatchSet::new(),
            backups: Vec::new(),
        }
    }

    pub fn add_proposal(&mut self, plan: &ChangePlan) {
        self.patches.add_patch(plan.patch.clone());
    }
}

impl Default for ChangeReview {
    fn default() -> Self {
        Self::new()
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
    fn test_apply_and_verify_rolls_back_on_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "keep\n").unwrap();

        let mut plan = ChangePlan::propose(&path, "evil\n").unwrap();
        let err = plan
            .apply_and_verify(Some("exit 1"))
            .expect_err("failing verification must roll back");
        assert!(err.to_string().contains("rolled back"), "{}", err);
        // Original content restored after rollback.
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep\n");
    }

    #[test]
    fn test_apply_and_verify_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "a\n").unwrap();

        let mut plan = ChangePlan::propose(&path, "b\n").unwrap();
        let result = plan
            .apply_and_verify(Some("true"))
            .expect("verification passing keeps change");
        assert!(result.contains("applied"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "b\n");
    }

    #[test]
    fn test_change_review_collects_patches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p1 = dir.path().join("a.txt");
        let p2 = dir.path().join("b.txt");
        fs::write(&p1, "1\n").unwrap();
        fs::write(&p2, "2\n").unwrap();

        let a = ChangePlan::propose(&p1, "11\n").unwrap();
        let b = ChangePlan::propose(&p2, "22\n").unwrap();

        let mut review = ChangeReview::new();
        review.add_proposal(&a);
        review.add_proposal(&b);
        assert_eq!(review.patches.patches.len(), 2);
    }

    #[test]
    fn test_propose_between_for_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("new.txt");
        let plan = ChangePlan::propose_between(&path, "", "hello\n").unwrap();
        assert!(plan.preview().contains("+hello"));
    }
}

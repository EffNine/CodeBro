#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use std::process::Command;

pub struct GitStatus;

impl super::Tool for GitStatus {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Show git working tree status"
    }

    fn execute(&self, _args: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["status", "--short"])
            .output()
            .with_context(|| "Failed to run git status")?;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // Tool output is fed to model context; redact any stray credential.
            Ok(crate::tools::shell::redact_secrets_public(&text))
        } else {
            Err(anyhow::anyhow!("git status failed"))
        }
    }
}

pub struct GitDiff;

impl super::Tool for GitDiff {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show git diff of changes"
    }

    fn execute(&self, _args: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["diff"])
            .output()
            .with_context(|| "Failed to run git diff")?;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // A diff can surface a committed/staged credential (e.g. an .env
            // change); redact before it reaches tool output / model context.
            Ok(crate::tools::shell::redact_secrets_public(&text))
        } else {
            Err(anyhow::anyhow!("git diff failed"))
        }
    }
}

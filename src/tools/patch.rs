#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use std::path::PathBuf;

use crate::error::{CodeBroError, Result};

#[derive(Debug, Clone)]
pub struct PatchHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FilePatch {
    pub path: PathBuf,
    pub hunks: Vec<PatchHunk>,
    pub unified_diff: String,
}

#[derive(Debug, Clone)]
pub struct PatchSet {
    pub patches: Vec<FilePatch>,
}

impl PatchSet {
    pub fn new() -> Self {
        PatchSet {
            patches: Vec::new(),
        }
    }

    pub fn add_patch(&mut self, patch: FilePatch) {
        self.patches.push(patch);
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

impl Default for PatchSet {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PatchEngine;

impl PatchEngine {
    pub fn create_patch(path: &PathBuf, old_content: &str, new_content: &str) -> Result<FilePatch> {
        let diff = Self::compute_diff(old_content, new_content)?;
        let hunks = Self::parse_hunks(&diff, old_content, new_content)?;
        let unified_diff = Self::generate_unified_diff(path, old_content, new_content)?;

        Ok(FilePatch {
            path: path.clone(),
            hunks,
            unified_diff,
        })
    }

    pub fn preview(patch: &FilePatch) -> &str {
        &patch.unified_diff
    }

    pub fn apply(patch: &FilePatch, dry_run: bool) -> Result<String> {
        if dry_run {
            return Ok(format!(
                "Dry run: Would apply patch to {}\n{}",
                patch.path.display(),
                patch.unified_diff
            ));
        }

        let new_content = Self::reconstruct_from_patch(
            &std::fs::read_to_string(&patch.path)
                .map_err(|e| CodeBroError::Patch(format!("Failed to read file: {}", e)))?,
            patch,
        )?;

        std::fs::write(&patch.path, new_content)
            .map_err(|e| CodeBroError::Patch(format!("Failed to write file: {}", e)))?;

        Ok(format!("Patch applied to {}", patch.path.display()))
    }

    pub fn rollback(path: &PathBuf, backup: &str) -> Result<()> {
        std::fs::write(path, backup)
            .map_err(|e| CodeBroError::Patch(format!("Failed to rollback: {}", e)))?;
        Ok(())
    }

    pub fn validate_patch(patch: &FilePatch) -> Result<()> {
        for hunk in &patch.hunks {
            if hunk.old_count == 0 && hunk.new_count == 0 {
                return Err(CodeBroError::Patch("Empty hunk detected".to_string()));
            }
        }
        Ok(())
    }

    fn compute_diff(old: &str, new: &str) -> Result<String> {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        let mut result = String::new();
        let mut old_idx = 0;
        let mut new_idx = 0;

        while old_idx < old_lines.len() || new_idx < new_lines.len() {
            if old_idx < old_lines.len()
                && new_idx < new_lines.len()
                && old_lines[old_idx] == new_lines[new_idx]
            {
                result.push_str(&format!(" {}\n", old_lines[old_idx]));
                old_idx += 1;
                new_idx += 1;
            } else {
                if old_idx < old_lines.len() {
                    result.push_str(&format!("-{}\n", old_lines[old_idx]));
                    old_idx += 1;
                }
                if new_idx < new_lines.len() {
                    result.push_str(&format!("+{}\n", new_lines[new_idx]));
                    new_idx += 1;
                }
            }
        }

        Ok(result)
    }

    fn parse_hunks(diff: &str, _old_content: &str, _new_content: &str) -> Result<Vec<PatchHunk>> {
        let mut hunks = Vec::new();
        let mut current_hunk = PatchHunk {
            old_start: 1,
            old_count: 0,
            new_start: 1,
            new_count: 0,
            lines: Vec::new(),
        };

        let mut _old_line = 1;
        let mut _new_line = 1;

        for line in diff.lines() {
            match line.chars().next() {
                Some(' ') => {
                    current_hunk.lines.push(line[1..].to_string());
                    _old_line += 1;
                    _new_line += 1;
                    current_hunk.old_count += 1;
                    current_hunk.new_count += 1;
                }
                Some('-') => {
                    current_hunk.lines.push(line.to_string());
                    _old_line += 1;
                    current_hunk.old_count += 1;
                }
                Some('+') => {
                    current_hunk.lines.push(line.to_string());
                    _new_line += 1;
                    current_hunk.new_count += 1;
                }
                _ => continue,
            }
        }

        if !current_hunk.lines.is_empty() {
            hunks.push(current_hunk);
        }

        Ok(hunks)
    }

    fn generate_unified_diff(path: &PathBuf, old: &str, new: &str) -> Result<String> {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        let _timestamp = chrono::Local::now().to_rfc3339();
        let mut diff = String::new();

        diff.push_str(&format!("--- a/{}\n", path.display()));
        diff.push_str(&format!("+++ b/{}\n", path.display()));
        diff.push_str(&format!(
            "@@ -1,{} +1,{} @@\n",
            old_lines.len(),
            new_lines.len()
        ));

        for (old_line, new_line) in old_lines.iter().zip(new_lines.iter()) {
            if old_line == new_line {
                diff.push_str(&format!(" {}\n", old_line));
            } else {
                diff.push_str(&format!("-{}\n", old_line));
                diff.push_str(&format!("+{}\n", new_line));
            }
        }

        if old_lines.len() > new_lines.len() {
            for line in &old_lines[new_lines.len()..] {
                diff.push_str(&format!("-{}\n", line));
            }
        } else if new_lines.len() > old_lines.len() {
            for line in &new_lines[old_lines.len()..] {
                diff.push_str(&format!("+{}\n", line));
            }
        }

        Ok(diff)
    }

    fn reconstruct_from_patch(original: &str, patch: &FilePatch) -> Result<String> {
        let mut result = original.to_string();
        let lines: Vec<&str> = original.lines().collect();

        for hunk in &patch.hunks {
            let start_idx = hunk.old_start.saturating_sub(1);
            let end_idx = (start_idx + hunk.old_count).min(lines.len());

            let mut new_lines = Vec::new();
            for line in &hunk.lines {
                if !line.starts_with('-') {
                    new_lines.push(if line.starts_with('+') {
                        line[1..].to_string()
                    } else {
                        line.to_string()
                    });
                }
            }

            let before: Vec<&str> = lines[..start_idx].to_vec();
            let after: Vec<&str> = lines[end_idx..].to_vec();

            result = String::new();
            for line in &before {
                result.push_str(line);
                result.push('\n');
            }
            for line in &new_lines {
                result.push_str(line);
                result.push('\n');
            }
            for line in &after {
                result.push_str(line);
                result.push('\n');
            }
        }

        Ok(result)
    }
}

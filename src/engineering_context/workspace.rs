//! Workspace context — filesystem and structural metadata about the
//! current working directory.

use serde::{Deserialize, Serialize};

/// Immutable description of the workspace root and its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceContext {
    /// Absolute or relative path to the workspace root.
    pub root_path: String,
    /// List of relevant file paths, sorted for determinism.
    pub relevant_files: Vec<WorkspaceFile>,
    /// Workspace-level metadata flags.
    pub has_git: bool,
    pub has_package_json: bool,
    pub has_cargo_toml: bool,
    pub has_readme: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub path: String,
    pub language: String,
    pub size_bytes: usize,
}

impl WorkspaceContext {
    pub fn new(root_path: impl Into<String>) -> Self {
        WorkspaceContext {
            root_path: root_path.into(),
            relevant_files: Vec::new(),
            has_git: false,
            has_package_json: false,
            has_cargo_toml: false,
            has_readme: false,
        }
    }

    pub fn with_file(mut self, file: WorkspaceFile) -> Self {
        self.relevant_files.push(file);
        self.relevant_files
            .sort_by(|a, b| a.path.cmp(&b.path));
        self
    }

    pub fn with_git(mut self, has: bool) -> Self {
        self.has_git = has;
        self
    }

    pub fn with_package_json(mut self, has: bool) -> Self {
        self.has_package_json = has;
        self
    }

    pub fn with_cargo_toml(mut self, has: bool) -> Self {
        self.has_cargo_toml = has;
        self
    }

    pub fn with_readme(mut self, has: bool) -> Self {
        self.has_readme = has;
        self
    }

    pub fn file_count(&self) -> usize {
        self.relevant_files.len()
    }

    pub fn total_size_bytes(&self) -> usize {
        self.relevant_files.iter().map(|f| f.size_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_workspace() {
        let ws = WorkspaceContext::new("/tmp/project");
        assert_eq!(ws.root_path, "/tmp/project");
        assert_eq!(ws.file_count(), 0);
        assert_eq!(ws.total_size_bytes(), 0);
    }

    #[test]
    fn test_workspace_with_files() {
        let ws = WorkspaceContext::new(".")
            .with_file(WorkspaceFile {
                path: "src/main.rs".to_string(),
                language: "rust".to_string(),
                size_bytes: 512,
            })
            .with_file(WorkspaceFile {
                path: "Cargo.toml".to_string(),
                language: "toml".to_string(),
                size_bytes: 128,
            })
            .with_git(true)
            .with_cargo_toml(true);

        assert_eq!(ws.file_count(), 2);
        assert_eq!(ws.total_size_bytes(), 640);
        assert!(ws.has_git);
        assert!(ws.has_cargo_toml);
        assert_eq!(ws.relevant_files[0].path, "Cargo.toml");
        assert_eq!(ws.relevant_files[1].path, "src/main.rs");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let ws = WorkspaceContext::new(".")
            .with_file(WorkspaceFile {
                path: "a.rs".to_string(),
                language: "rust".to_string(),
                size_bytes: 100,
            })
            .with_git(true);
        let json = serde_json::to_string(&ws).expect("serialize");
        let decoded: WorkspaceContext = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(ws, decoded);
    }
}

//! Project identity — deterministic, serialisable metadata about the
//! repository being worked on.

use serde::{Deserialize, Serialize};

/// Immutable description of the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIdentity {
    /// Human-readable project name.
    pub name: String,
    /// Primary programming language.
    pub language: String,
    /// Optional framework identifier (e.g. `"axum"`, `"react"`).
    pub framework: Option<String>,
    /// Optional build system (e.g. `"cargo"`, `"make"`).
    pub build_system: Option<String>,
    /// Optional package manager (e.g. `"cargo"`, `"npm"`).
    pub package_manager: Option<String>,
    /// Optional testing framework (e.g. `"cargo test"`, `"jest"`).
    pub testing_framework: Option<String>,
    /// Paths of project-critical files, sorted alphabetically for determinism.
    pub important_files: Vec<String>,
}

impl ProjectIdentity {
    pub fn new(name: impl Into<String>, language: impl Into<String>) -> Self {
        ProjectIdentity {
            name: name.into(),
            language: language.into(),
            framework: None,
            build_system: None,
            package_manager: None,
            testing_framework: None,
            important_files: Vec::new(),
        }
    }

    pub fn with_framework(mut self, framework: impl Into<String>) -> Self {
        self.framework = Some(framework.into());
        self
    }

    pub fn with_build_system(mut self, system: impl Into<String>) -> Self {
        self.build_system = Some(system.into());
        self
    }

    pub fn with_package_manager(mut self, pm: impl Into<String>) -> Self {
        self.package_manager = Some(pm.into());
        self
    }

    pub fn with_testing_framework(mut self, tf: impl Into<String>) -> Self {
        self.testing_framework = Some(tf.into());
        self
    }

    pub fn with_important_files(mut self, files: Vec<String>) -> Self {
        self.important_files = files;
        self.important_files.sort();
        self
    }

    pub fn add_important_file(mut self, file: impl Into<String>) -> Self {
        self.important_files.push(file.into());
        self.important_files.sort();
        self
    }

    /// Returns `true` when the identity has no optional fields set
    /// and only the bare minimum (`name` + `language`) is present.
    pub fn is_basic(&self) -> bool {
        self.framework.is_none()
            && self.build_system.is_none()
            && self.package_manager.is_none()
            && self.testing_framework.is_none()
            && self.important_files.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_identity() {
        let id = ProjectIdentity::new("my-project", "rust");
        assert_eq!(id.name, "my-project");
        assert_eq!(id.language, "rust");
        assert!(id.is_basic());
    }

    #[test]
    fn test_full_identity() {
        let id = ProjectIdentity::new("web-app", "typescript")
            .with_framework("nextjs")
            .with_build_system("npm")
            .with_package_manager("npm")
            .with_testing_framework("vitest")
            .with_important_files(vec!["app.tsx".to_string(), "package.json".to_string()]);

        assert_eq!(id.framework, Some("nextjs".to_string()));
        assert!(!id.is_basic());
        assert_eq!(id.important_files, vec!["app.tsx".to_string(), "package.json".to_string()]);
    }

    #[test]
    fn test_deterministic_sort() {
        let id = ProjectIdentity::new("proj", "rust")
            .add_important_file("z.rs")
            .add_important_file("a.rs")
            .add_important_file("m.rs");
        assert_eq!(id.important_files, vec!["a.rs", "m.rs", "z.rs"]);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let id = ProjectIdentity::new("proj", "go")
            .with_framework("gin")
            .with_build_system("go build")
            .add_important_file("main.go");
        let json = serde_json::to_string(&id).expect("serialize");
        let decoded: ProjectIdentity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, decoded);
    }
}

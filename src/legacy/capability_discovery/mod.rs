//! Capability Discovery
//!
//! Auto-detects available tools, capabilities, and features in the current
//! workspace. Presents recommendations to the user for enabling.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityKind {
    /// Tool capabilities
    Tool(String),
    /// Language runtime
    LanguageRuntime(String),
    /// Build system
    BuildSystem(String),
    /// Testing framework
    TestingFramework(String),
    /// IDE / editor integration
    EditorIntegration(String),
    /// MCP server
    McpServer(String),
}

impl std::fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityKind::Tool(t) => write!(f, "Tool: {}", t),
            CapabilityKind::LanguageRuntime(lang) => write!(f, "Runtime: {}", lang),
            CapabilityKind::BuildSystem(bs) => write!(f, "Build: {}", bs),
            CapabilityKind::TestingFramework(tf) => write!(f, "Testing: {}", tf),
            CapabilityKind::EditorIntegration(ei) => write!(f, "Editor: {}", ei),
            CapabilityKind::McpServer(ms) => write!(f, "MCP: {}", ms),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub kind: CapabilityKind,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub enabled: bool,
    pub recommendation: Recommendation,
    pub confidence: f32,
}

impl Capability {
    pub fn new(kind: CapabilityKind, name: String, description: String) -> Self {
        Capability {
            kind,
            name,
            description,
            available: false,
            enabled: false,
            recommendation: Recommendation::None,
            confidence: 0.0,
        }
    }

    pub fn available(name: String, description: String, confidence: f32) -> Self {
        Capability {
            kind: CapabilityKind::Tool(name.clone()),
            name,
            description,
            available: true,
            enabled: false,
            recommendation: Recommendation::Recommended,
            confidence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Recommendation {
    None,
    Recommended,
    Optional,
    Required,
}

impl std::fmt::Display for Recommendation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Recommendation::None => write!(f, "—"),
            Recommendation::Recommended => write!(f, "Recommended"),
            Recommendation::Optional => write!(f, "Optional"),
            Recommendation::Required => write!(f, "Required"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDiscovery {
    pub capabilities: Vec<Capability>,
    pub recommendations: Vec<String>,
    pub workspace_root: PathBuf,
}

impl CapabilityDiscovery {
    pub fn new(workspace_root: PathBuf) -> Self {
        CapabilityDiscovery {
            capabilities: Vec::new(),
            recommendations: Vec::new(),
            workspace_root,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    pub fn available_count(&self) -> usize {
        self.capabilities.iter().filter(|c| c.available).count()
    }

    pub fn enabled_count(&self) -> usize {
        self.capabilities.iter().filter(|c| c.enabled).count()
    }

    pub fn add_capability(&mut self, cap: Capability) {
        self.capabilities.push(cap);
    }

    pub fn add_recommendation(&mut self, rec: String) {
        if !self.recommendations.contains(&rec) {
            self.recommendations.push(rec);
        }
    }

    pub fn enable_recommended(&mut self) {
        for cap in &mut self.capabilities {
            if cap.available
                && matches!(
                    cap.recommendation,
                    Recommendation::Recommended | Recommendation::Required
                )
            {
                cap.enabled = true;
            }
        }
    }

    pub fn summary_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Capabilities: {} available, {} enabled",
            self.available_count(),
            self.enabled_count()
        ));

        for cap in &self.capabilities {
            if cap.available {
                let icon = if cap.enabled { "✓" } else { "○" };
                lines.push(format!(
                    "  {} {} [{}] - {}",
                    icon, cap.name, cap.recommendation, cap.description
                ));
            }
        }

        if !self.recommendations.is_empty() {
            lines.push(String::new());
            lines.push("Recommendations:".to_string());
            for rec in &self.recommendations {
                lines.push(format!("  • {}", rec));
            }
        }

        lines.join("\n")
    }
}

// ─── Capability Scanner ──────────────────────────────────────────────────────

#[derive(Default)]
pub struct CapabilityScanner {
    workspace_root: PathBuf,
}

impl CapabilityScanner {
    pub fn new(workspace_root: PathBuf) -> Self {
        CapabilityScanner { workspace_root }
    }

    pub fn scan(&self) -> CapabilityDiscovery {
        let mut discovery = CapabilityDiscovery::new(self.workspace_root.clone());

        self.scan_tools(&mut discovery);
        self.scan_runtimes(&mut discovery);
        self.scan_build_systems(&mut discovery);
        self.scan_testing(&mut discovery);
        self.scan_editors(&mut discovery);

        // Generate recommendations
        self.generate_recommendations(&mut discovery);

        discovery
    }

    fn scan_tools(&self, discovery: &mut CapabilityDiscovery) {
        // Check for built-in tools that are always available
        discovery.add_capability(Capability::available(
            "read_file".to_string(),
            "Read files from the workspace".to_string(),
            1.0,
        ));
        discovery.add_capability(Capability::available(
            "edit_file".to_string(),
            "Edit files with patch-based changes".to_string(),
            1.0,
        ));
        discovery.add_capability(Capability::available(
            "run_command".to_string(),
            "Execute shell commands with timeout".to_string(),
            1.0,
        ));
        discovery.add_capability(Capability::available(
            "git_status".to_string(),
            "Get git status and diff".to_string(),
            1.0,
        ));
    }

    fn scan_runtimes(&self, discovery: &mut CapabilityDiscovery) {
        let root = &self.workspace_root;

        if root.join("Cargo.toml").exists() {
            discovery.add_capability(Capability {
                kind: CapabilityKind::LanguageRuntime("rust".to_string()),
                name: "rustc".to_string(),
                description: "Rust compiler and Cargo".to_string(),
                available: true,
                enabled: false,
                recommendation: Recommendation::Required,
                confidence: 1.0,
            });
        }

        if root.join("package.json").exists() {
            discovery.add_capability(Capability {
                kind: CapabilityKind::LanguageRuntime("javascript".to_string()),
                name: "node".to_string(),
                description: "Node.js runtime".to_string(),
                available: true,
                enabled: false,
                recommendation: Recommendation::Recommended,
                confidence: 1.0,
            });
        }

        if root.join("pyproject.toml").exists() || root.join("requirements.txt").exists() {
            discovery.add_capability(Capability {
                kind: CapabilityKind::LanguageRuntime("python".to_string()),
                name: "python".to_string(),
                description: "Python runtime".to_string(),
                available: true,
                enabled: false,
                recommendation: Recommendation::Recommended,
                confidence: 1.0,
            });
        }

        if root.join("go.mod").exists() {
            discovery.add_capability(Capability {
                kind: CapabilityKind::LanguageRuntime("go".to_string()),
                name: "go".to_string(),
                description: "Go runtime".to_string(),
                available: true,
                enabled: false,
                recommendation: Recommendation::Required,
                confidence: 1.0,
            });
        }
    }

    fn scan_build_systems(&self, discovery: &mut CapabilityDiscovery) {
        let root = &self.workspace_root;

        if root.join("Cargo.toml").exists() {
            discovery.add_capability(Capability::new(
                CapabilityKind::BuildSystem("cargo".to_string()),
                "cargo".to_string(),
                "Cargo build and test system".to_string(),
            ));
        }

        if root.join("package.json").exists() {
            discovery.add_capability(Capability::new(
                CapabilityKind::BuildSystem("npm".to_string()),
                "npm".to_string(),
                "npm build system".to_string(),
            ));
        }

        if root.join("Makefile").exists() || root.join("makefile").exists() {
            discovery.add_capability(Capability::new(
                CapabilityKind::BuildSystem("make".to_string()),
                "make".to_string(),
                "Make build system".to_string(),
            ));
        }
    }

    fn scan_testing(&self, discovery: &mut CapabilityDiscovery) {
        let root = &self.workspace_root;

        if root.join("Cargo.toml").exists() {
            discovery.add_capability(Capability::new(
                CapabilityKind::TestingFramework("cargo_test".to_string()),
                "cargo test".to_string(),
                "Rust test runner via Cargo".to_string(),
            ));
        }

        if root.join("package.json").exists() {
            if let Ok(content) = std::fs::read_to_string(root.join("package.json")) {
                if content.contains("jest") {
                    discovery.add_capability(Capability::new(
                        CapabilityKind::TestingFramework("jest".to_string()),
                        "jest".to_string(),
                        "Jest test framework".to_string(),
                    ));
                }
                if content.contains("vitest") {
                    discovery.add_capability(Capability::new(
                        CapabilityKind::TestingFramework("vitest".to_string()),
                        "vitest".to_string(),
                        "Vitest test framework".to_string(),
                    ));
                }
            }
        }

        if root.join("pytest.ini").exists() || root.join("conftest.py").exists() {
            discovery.add_capability(Capability::new(
                CapabilityKind::TestingFramework("pytest".to_string()),
                "pytest".to_string(),
                "Pytest test framework".to_string(),
            ));
        }
    }

    fn scan_editors(&self, discovery: &mut CapabilityDiscovery) {
        // Check for common editor config files
        let root = &self.workspace_root;

        if root.join(".vscode").exists() {
            discovery.add_capability(Capability {
                kind: CapabilityKind::EditorIntegration("vscode".to_string()),
                name: "VS Code".to_string(),
                description: "VS Code workspace detected".to_string(),
                available: true,
                enabled: false,
                recommendation: Recommendation::Optional,
                confidence: 0.8,
            });
        }

        if root.join(".claude").exists() || root.join(".cursorrules").exists() {
            discovery.add_capability(Capability {
                kind: CapabilityKind::EditorIntegration("ai_editor".to_string()),
                name: "AI Editor Config".to_string(),
                description: "AI editor configuration detected".to_string(),
                available: true,
                enabled: false,
                recommendation: Recommendation::Optional,
                confidence: 0.7,
            });
        }
    }

    fn generate_recommendations(&self, discovery: &mut CapabilityDiscovery) {
        let rust = discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, CapabilityKind::LanguageRuntime(r) if r == "rust") && c.available
        });
        let node = discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, CapabilityKind::LanguageRuntime(n) if n == "javascript")
                && c.available
        });

        if rust {
            discovery.add_recommendation("Enable cargo test runner for Rust projects".to_string());
        }
        if node {
            discovery
                .add_recommendation("Enable npm script execution for Node.js projects".to_string());
        }

        if discovery.capabilities.iter().any(|c| {
            matches!(&c.kind, CapabilityKind::BuildSystem(bs) if bs == "cargo") && c.available
        }) {
            discovery.add_recommendation("Use Cargo for build and test operations".to_string());
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_display() {
        let cap = Capability::new(
            CapabilityKind::Tool("test".to_string()),
            "test_tool".to_string(),
            "A test tool".to_string(),
        );
        assert_eq!(format!("{}", cap.kind), "Tool: test");
    }

    #[test]
    fn test_recommendation_display() {
        assert_eq!(format!("{}", Recommendation::Recommended), "Recommended");
        assert_eq!(format!("{}", Recommendation::None), "—");
    }

    #[test]
    fn test_capability_discovery_empty() {
        let scanner = CapabilityScanner::new(PathBuf::from("/tmp"));
        let discovery = scanner.scan();
        assert!(!discovery.is_empty()); // At least built-in tools
        assert!(discovery.available_count() >= 4); // read_file, edit_file, run_command, git_status
    }

    #[test]
    fn test_summary_text() {
        let mut discovery = CapabilityDiscovery::new(PathBuf::from("/tmp"));
        discovery.add_capability(Capability::available(
            "tool_a".to_string(),
            "Tool A".to_string(),
            1.0,
        ));
        discovery.add_recommendation("Use tool_a for best results".to_string());
        let summary = discovery.summary_text();
        assert!(summary.contains("tool_a"));
        assert!(summary.contains("Use tool_a"));
    }
}

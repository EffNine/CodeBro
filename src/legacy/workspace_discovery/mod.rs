//! Workspace Discovery
//!
//! Automatically detects project structure, build systems, package managers,
//! and available integrations. Presents findings to the user for approval
//! before enabling any integration.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Discovery Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveryKind {
    Git,
    Cargo,
    Npm,
    Python,
    Docker,
    Go,
    Ruby,
    Php,
    Java,
    Bun,
    Pnpm,
    Yarn,
    Make,
    Cmake,
}

impl std::fmt::Display for DiscoveryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryKind::Git => write!(f, "Git"),
            DiscoveryKind::Cargo => write!(f, "Cargo (Rust)"),
            DiscoveryKind::Npm => write!(f, "npm (Node.js)"),
            DiscoveryKind::Python => write!(f, "Python"),
            DiscoveryKind::Docker => write!(f, "Docker"),
            DiscoveryKind::Go => write!(f, "Go"),
            DiscoveryKind::Ruby => write!(f, "Ruby"),
            DiscoveryKind::Php => write!(f, "PHP"),
            DiscoveryKind::Java => write!(f, "Java"),
            DiscoveryKind::Bun => write!(f, "Bun"),
            DiscoveryKind::Pnpm => write!(f, "pnpm"),
            DiscoveryKind::Yarn => write!(f, "Yarn"),
            DiscoveryKind::Make => write!(f, "Make"),
            DiscoveryKind::Cmake => write!(f, "CMake"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryFinding {
    pub kind: DiscoveryKind,
    pub description: String,
    pub confidence: f32, // 0.0 to 1.0
    pub details: Vec<String>,
}

impl DiscoveryFinding {
    pub fn new(kind: DiscoveryKind, description: String, confidence: f32) -> Self {
        DiscoveryFinding {
            kind,
            description,
            confidence,
            details: Vec::new(),
        }
    }

    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationProposal {
    pub name: String,
    pub finding: DiscoveryFinding,
    pub enabled: bool,
    pub approved: bool,
    pub requires_approval: bool,
}

impl IntegrationProposal {
    pub fn new(name: String, finding: DiscoveryFinding) -> Self {
        IntegrationProposal {
            name,
            finding,
            enabled: false,
            approved: false,
            requires_approval: true,
        }
    }

    pub fn auto_enable(name: String, finding: DiscoveryFinding) -> Self {
        IntegrationProposal {
            name,
            finding,
            enabled: true,
            approved: true,
            requires_approval: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDiscovery {
    pub root: PathBuf,
    pub findings: Vec<DiscoveryFinding>,
    pub proposals: Vec<IntegrationProposal>,
    pub language: String,
    pub framework: Option<String>,
    pub build_system: Option<String>,
    pub package_manager: Option<String>,
    pub testing_framework: Option<String>,
}

impl WorkspaceDiscovery {
    pub fn new(root: PathBuf) -> Self {
        WorkspaceDiscovery {
            root: root.clone(),
            findings: Vec::new(),
            proposals: Vec::new(),
            language: "unknown".to_string(),
            framework: None,
            build_system: None,
            package_manager: None,
            testing_framework: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn enabled_count(&self) -> usize {
        self.proposals.iter().filter(|p| p.enabled).count()
    }

    pub fn requires_approval_count(&self) -> usize {
        self.proposals
            .iter()
            .filter(|p| p.requires_approval && !p.approved)
            .count()
    }
}

// ─── Discovery Engine ────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DiscoveryEngine {
    root: PathBuf,
}

impl DiscoveryEngine {
    pub fn new(root: PathBuf) -> Self {
        DiscoveryEngine { root }
    }

    pub fn discover(&self) -> WorkspaceDiscovery {
        let mut discovery = WorkspaceDiscovery::new(self.root.clone());

        self.discover_git(&mut discovery);
        self.discover_language(&mut discovery);
        self.discover_build_system(&mut discovery);
        self.discover_package_manager(&mut discovery);
        self.discover_testing(&mut discovery);
        self.discover_docker(&mut discovery);
        self.discover_integrations(&mut discovery);

        discovery
    }

    fn discover_git(&self, discovery: &mut WorkspaceDiscovery) {
        if self.root.join(".git").exists() {
            let mut finding = DiscoveryFinding::new(
                DiscoveryKind::Git,
                "Git version control detected".to_string(),
                1.0,
            );
            finding.details.push("Repository initialized".to_string());
            if let Ok(output) = std::process::Command::new("git")
                .arg("remote")
                .arg("get-url")
                .arg("origin")
                .output()
            {
                if output.status.success() {
                    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !url.is_empty() {
                        finding.details.push(format!("Remote: {}", url));
                    }
                }
            }
            discovery.findings.push(finding);
        }
    }

    fn discover_language(&self, discovery: &mut WorkspaceDiscovery) {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return,
        };

        let file_names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        // Language detection
        if file_names.contains(&"Cargo.toml".to_string()) {
            discovery.language = "rust".to_string();
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Cargo,
                    "Rust project (Cargo.toml)".to_string(),
                    1.0,
                )
                .with_details(vec!["Language: Rust".to_string()]),
            );
        } else if file_names.contains(&"package.json".to_string()) {
            discovery.language = "javascript".to_string();
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Npm,
                    "Node.js project (package.json)".to_string(),
                    1.0,
                )
                .with_details(vec!["Language: JavaScript/TypeScript".to_string()]),
            );
        } else if file_names.contains(&"pyproject.toml".to_string())
            || file_names.contains(&"requirements.txt".to_string())
        {
            discovery.language = "python".to_string();
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Python,
                    "Python project detected".to_string(),
                    1.0,
                )
                .with_details(vec!["Language: Python".to_string()]),
            );
        } else if file_names.contains(&"go.mod".to_string()) {
            discovery.language = "go".to_string();
            discovery.findings.push(
                DiscoveryFinding::new(DiscoveryKind::Go, "Go project (go.mod)".to_string(), 1.0)
                    .with_details(vec!["Language: Go".to_string()]),
            );
        }

        // Framework detection
        if let Ok(content) = std::fs::read_to_string(self.root.join("package.json")) {
            if content.contains("\"react\"") {
                discovery.framework = Some("react".to_string());
            } else if content.contains("\"next\"") {
                discovery.framework = Some("next.js".to_string());
            } else if content.contains("\"vue\"") {
                discovery.framework = Some("vue".to_string());
            }
        }

        if let Ok(content) = std::fs::read_to_string(self.root.join("Cargo.toml")) {
            if content.contains("actix-web") || content.contains("actix_web") {
                discovery.framework = Some("actix-web".to_string());
            } else if content.contains("axum") {
                discovery.framework = Some("axum".to_string());
            }
        }
    }

    fn discover_build_system(&self, discovery: &mut WorkspaceDiscovery) {
        if self.root.join("Cargo.toml").exists() {
            discovery.build_system = Some("cargo".to_string());
        } else if self.root.join("package.json").exists() {
            discovery.build_system = Some("npm".to_string());
        } else if self.root.join("Makefile").exists() || self.root.join("makefile").exists() {
            discovery.build_system = Some("make".to_string());
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Make,
                    "Make build system detected".to_string(),
                    1.0,
                )
                .with_details(vec!["Makefile found".to_string()]),
            );
        } else if self.root.join("CMakeLists.txt").exists() {
            discovery.build_system = Some("cmake".to_string());
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Cmake,
                    "CMake build system detected".to_string(),
                    1.0,
                )
                .with_details(vec!["CMakeLists.txt found".to_string()]),
            );
        }
    }

    fn discover_package_manager(&self, discovery: &mut WorkspaceDiscovery) {
        if self.root.join("Cargo.toml").exists() {
            discovery.package_manager = Some("cargo".to_string());
        } else if self.root.join("pnpm-lock.yaml").exists() {
            discovery.package_manager = Some("pnpm".to_string());
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Pnpm,
                    "pnpm package manager detected".to_string(),
                    1.0,
                )
                .with_details(vec!["pnpm-lock.yaml found".to_string()]),
            );
        } else if self.root.join("yarn.lock").exists() {
            discovery.package_manager = Some("yarn".to_string());
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Yarn,
                    "Yarn package manager detected".to_string(),
                    1.0,
                )
                .with_details(vec!["yarn.lock found".to_string()]),
            );
        } else if self.root.join("package-lock.json").exists() {
            discovery.package_manager = Some("npm".to_string());
        } else if self.root.join("bun.lockb").exists() || self.root.join("bun.lock").exists() {
            discovery.package_manager = Some("bun".to_string());
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Bun,
                    "Bun package manager detected".to_string(),
                    1.0,
                )
                .with_details(vec!["bun lockfile found".to_string()]),
            );
        }
    }

    fn discover_testing(&self, discovery: &mut WorkspaceDiscovery) {
        if discovery.language == "rust" {
            discovery.testing_framework = Some("cargo_test".to_string());
        } else if discovery.language == "javascript" {
            if let Ok(content) = std::fs::read_to_string(self.root.join("package.json")) {
                if content.contains("jest") {
                    discovery.testing_framework = Some("jest".to_string());
                } else if content.contains("vitest") {
                    discovery.testing_framework = Some("vitest".to_string());
                }
            }
        } else if discovery.language == "python" {
            if self.root.join("pytest.ini").exists() || self.root.join("conftest.py").exists() {
                discovery.testing_framework = Some("pytest".to_string());
            }
        }
    }

    fn discover_docker(&self, discovery: &mut WorkspaceDiscovery) {
        let dockerfile = ["Dockerfile", "dockerfile", "Dockerfile.dev"];
        let compose = ["docker-compose.yml", "docker-compose.yaml", "compose.yml"];

        let has_dockerfile = dockerfile.iter().any(|f| self.root.join(f).exists());
        let has_compose = compose.iter().any(|f| self.root.join(f).exists());

        if has_dockerfile || has_compose {
            discovery.findings.push(
                DiscoveryFinding::new(
                    DiscoveryKind::Docker,
                    "Docker support detected".to_string(),
                    1.0,
                )
                .with_details(
                    vec![
                        if has_dockerfile {
                            "Dockerfile found".to_string()
                        } else {
                            String::new()
                        },
                        if has_compose {
                            "docker-compose.yml found".to_string()
                        } else {
                            String::new()
                        },
                    ]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect(),
                ),
            );
        }
    }

    fn discover_integrations(&self, discovery: &mut WorkspaceDiscovery) {
        // Git integration
        if self.root.join(".git").exists() {
            discovery.proposals.push(IntegrationProposal::new(
                "Git Status & Diff".to_string(),
                discovery
                    .findings
                    .iter()
                    .find(|f| matches!(f.kind, DiscoveryKind::Git))
                    .cloned()
                    .unwrap_or_else(|| {
                        DiscoveryFinding::new(
                            DiscoveryKind::Git,
                            "Git version control".to_string(),
                            1.0,
                        )
                    }),
            ));
        }

        // Build system integration
        if let Some(ref bs) = discovery.build_system {
            let kind = match bs.as_str() {
                "cargo" => DiscoveryKind::Cargo,
                "npm" => DiscoveryKind::Npm,
                "make" => DiscoveryKind::Make,
                "cmake" => DiscoveryKind::Cmake,
                _ => DiscoveryKind::Cargo,
            };
            let finding = discovery
                .findings
                .iter()
                .find(|f| f.kind == kind)
                .cloned()
                .unwrap_or_else(|| {
                    DiscoveryFinding::new(kind, format!("{} build system", bs), 1.0)
                });
            discovery.proposals.push(IntegrationProposal::new(
                format!("{} Integration", bs),
                finding,
            ));
        }

        // Docker integration
        if discovery
            .findings
            .iter()
            .any(|f| matches!(f.kind, DiscoveryKind::Docker))
        {
            let finding = discovery
                .findings
                .iter()
                .find(|f| matches!(f.kind, DiscoveryKind::Docker))
                .cloned()
                .unwrap_or_else(|| {
                    DiscoveryFinding::new(DiscoveryKind::Docker, "Docker support".to_string(), 1.0)
                });
            discovery.proposals.push(IntegrationProposal::new(
                "Docker Build & Run".to_string(),
                finding,
            ));
        }
    }
}

// ─── MCP Discovery (Discovery Only, P5) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub available: bool,
    pub config_path: Option<PathBuf>,
}

pub fn discover_mcp_servers(root: &Path) -> Vec<McpServerInfo> {
    let mut servers = Vec::new();

    // Check .codebro/mcp.json
    let mcp_config = root.join(".codebro").join("mcp.json");
    if mcp_config.exists() {
        if let Ok(content) = std::fs::read_to_string(&mcp_config) {
            if let Ok(config) = serde_json::from_str::<McpConfig>(&content) {
                for (name, server) in config.servers {
                    servers.push(McpServerInfo {
                        name,
                        command: server.command,
                        args: server.args,
                        available: false, // Will be checked asynchronously
                        config_path: Some(mcp_config.clone()),
                    });
                }
            }
        }
    }

    // Check for common MCP server binaries
    // MCP server discovery is for display only in P5
    let _ = ["npx", "npm", "pip"];

    servers
}

fn get_known_mcp_servers(_invoker: &str) -> Vec<McpServerInfo> {
    vec![]
}

#[derive(Debug, Deserialize)]
struct McpConfig {
    servers: HashMap<String, McpServerEntry>,
}

#[derive(Debug, Deserialize)]
struct McpServerEntry {
    command: String,
    args: Vec<String>,
}

use std::collections::HashMap;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_kind_display() {
        assert_eq!(DiscoveryKind::Git.to_string(), "Git");
        assert_eq!(DiscoveryKind::Cargo.to_string(), "Cargo (Rust)");
        assert_eq!(DiscoveryKind::Docker.to_string(), "Docker");
    }

    #[test]
    fn test_integration_proposal() {
        let finding = DiscoveryFinding::new(DiscoveryKind::Git, "Git detected".to_string(), 1.0);
        let proposal = IntegrationProposal::new("Git Status".to_string(), finding);
        assert!(!proposal.enabled);
        assert!(proposal.requires_approval);
    }

    #[test]
    fn test_workspace_discovery_empty() {
        let engine = DiscoveryEngine::new(PathBuf::from("/tmp"));
        let discovery = engine.discover();
        assert_eq!(discovery.language, "unknown");
        assert!(discovery.is_empty() || discovery.findings.is_empty());
    }
}

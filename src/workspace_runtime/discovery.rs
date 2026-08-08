#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Build System & Package Manager Discovery (P10.4).
//!
//! Static, heuristic detection of the tooling a workspace uses. Discovery
//! is intentionally **breadth-first and shallow**: it inspects only well
//! known marker files in the root, never the full tree, and never reads
//! large content.
//!
//! Discovery is **observation only** — it returns data and never mutates
//! the workspace.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::workspace_runtime::context::WorkspaceRoot;

/// Tools the discovery layer can recognize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolKind {
    Cargo,
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Python,
    Pip,
    Pdm,
    Poetry,
    Go,
    Gradle,
    Maven,
    Make,
    Cmake,
    Docker,
}

impl std::fmt::Display for ToolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ToolKind::Cargo => "cargo",
            ToolKind::Npm => "npm",
            ToolKind::Pnpm => "pnpm",
            ToolKind::Yarn => "yarn",
            ToolKind::Bun => "bun",
            ToolKind::Python => "python",
            ToolKind::Pip => "pip",
            ToolKind::Pdm => "pdm",
            ToolKind::Poetry => "poetry",
            ToolKind::Go => "go",
            ToolKind::Gradle => "gradle",
            ToolKind::Maven => "maven",
            ToolKind::Make => "make",
            ToolKind::Cmake => "cmake",
            ToolKind::Docker => "docker",
        };
        write!(f, "{s}")
    }
}

/// A recognized build system with the tool and a confidence score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildSystemInfo {
    pub tool: ToolKind,
    pub confidence: f32,
    pub markers: Vec<String>,
}

/// A recognized package manager with its lockfile marker(s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageManagerInfo {
    pub tool: ToolKind,
    pub markers: Vec<String>,
}

/// Result of static discovery for a workspace root.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub root: WorkspaceRoot,
    /// Language hint synthesised from markers.
    pub language: Option<String>,
    pub build_system: Option<BuildSystemInfo>,
    pub package_manager: Option<PackageManagerInfo>,
    /// Marker files observed this pass.
    pub discovered_markers: Vec<String>,
}

impl DiscoveryReport {
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }
    pub fn build_tool(&self) -> Option<ToolKind> {
        self.build_system.as_ref().map(|b| b.tool)
    }
    pub fn package_tool(&self) -> Option<ToolKind> {
        self.package_manager.as_ref().map(|p| p.tool)
    }
}

/// The static discovery engine. Cheap, shallow, deterministic.
pub struct DiscoveryEngine;

impl DiscoveryEngine {
    /// Run discovery against a workspace root.
    ///
    /// Only inspects marker files at the root level. Never recurses, never
    /// reads file contents beyond marker names.
    pub fn discover(root: &WorkspaceRoot) -> DiscoveryReport {
        let root_path = &root.0;
        let mut report = DiscoveryReport {
            build_system: None,
            package_manager: None,
            language: None,
            root: root.clone(),
            discovered_markers: Vec::new(),
        };

        let has = |name: &str| root_path.join(name).exists();

        // ---- Build systems -------------------------------------------------
        if has("Cargo.toml") {
            report.language = Some("rust".into());
            report.build_system = Some(BuildSystemInfo {
                tool: ToolKind::Cargo,
                confidence: 1.0,
                markers: vec!["Cargo.toml".into()],
            });
            report.discovered_markers.push("Cargo.toml".into());
        } else if has("go.mod") {
            report.language = Some("go".into());
            report.build_system = Some(BuildSystemInfo {
                tool: ToolKind::Go,
                confidence: 1.0,
                markers: vec!["go.mod".into()],
            });
            report.discovered_markers.push("go.mod".into());
        } else if has("pyproject.toml") || has("requirements.txt") {
            report.language = Some("python".into());
            for m in ["pyproject.toml", "requirements.txt"] {
                if has(m) {
                    report.discovered_markers.push(m.into());
                }
            }
        } else if has("package.json") {
            report.language = Some("javascript".into());
            report.discovered_markers.push("package.json".into());
        } else if has("Makefile") || has("makefile") {
            report.build_system = Some(BuildSystemInfo {
                tool: ToolKind::Make,
                confidence: 0.8,
                markers: vec!["Makefile".into()],
            });
            report.discovered_markers.push("Makefile".into());
        } else if has("CMakeLists.txt") {
            report.build_system = Some(BuildSystemInfo {
                tool: ToolKind::Cmake,
                confidence: 0.9,
                markers: vec!["CMakeLists.txt".into()],
            });
            report.discovered_markers.push("CMakeLists.txt".into());
        }

        // ---- Package managers ---------------------------------------------
        let pm = Self::detect_package_manager(has);
        if let Some(pm) = pm {
            report.package_manager = Some(pm);
        }
        report
    }

    fn detect_package_manager<F: Fn(&str) -> bool>(has: F) -> Option<PackageManagerInfo> {
        // Lockfiles give the strongest signal; manifests are a fallback.
        if has("Cargo.lock") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Cargo,
                markers: vec!["Cargo.lock".into()],
            });
        }
        if has("Cargo.toml") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Cargo,
                markers: vec!["Cargo.toml".into()],
            });
        }
        if has("pnpm-lock.yaml") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Pnpm,
                markers: vec!["pnpm-lock.yaml".into()],
            });
        }
        if has("yarn.lock") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Yarn,
                markers: vec!["yarn.lock".into()],
            });
        }
        if has("bun.lockb") || has("bun.lock") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Bun,
                markers: vec!["bun.lockb".into()],
            });
        }
        if has("package-lock.json") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Npm,
                markers: vec!["package-lock.json".into()],
            });
        }
        if has("package.json") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Npm,
                markers: vec!["package.json".into()],
            });
        }
        if has("Pipfile") || has("poetry.lock") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Pip,
                markers: vec!["Pipfile".into()],
            });
        }
        if has("setup.py") || has("pyproject.toml") {
            return Some(PackageManagerInfo {
                tool: ToolKind::Pip,
                markers: vec!["setup.py".into()],
            });
        }
        None
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Environment Detection (P10.4).
//!
//! Detects the ambient execution environment: OS, architecture, running
//! under a container/CI, and which toolchains are available on PATH.
//!
//! Detection is **observation only** and must complete far under the cold
//! start budget. It never probes the project for content.

use serde::{Deserialize, Serialize};

/// Operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
    Other,
}

/// Architecture family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arch {
    X86_64,
    Aarch64,
    Arm,
    Other,
}

/// The result of environment detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentProfile {
    pub os: Os,
    pub arch: Arch,
    /// Whether we appear to run in a CI system.
    pub ci: bool,
    /// Whether we appear to run in a container.
    pub container: bool,
    /// A set of toolchains found on PATH.
    pub available_tools: Vec<String>,
    /// The user shell, when discoverable.
    pub shell: Option<String>,
    /// The terminal type, when discoverable.
    pub term: Option<String>,
}

impl Default for EnvironmentProfile {
    fn default() -> Self {
        EnvironmentProfile {
            os: Os::Other,
            arch: Arch::X86_64,
            ci: false,
            container: false,
            available_tools: Vec::new(),
            shell: None,
            term: None,
        }
    }
}

impl EnvironmentProfile {
    pub fn os(&self) -> Os {
        self.os
    }
    pub fn is_ci(&self) -> bool {
        self.ci
    }
    pub fn has_tool(&self, tool: &str) -> bool {
        self.available_tools.iter().any(|t| t == tool)
    }
}

/// Environment detector. Pure observation, cheap.
pub struct EnvironmentDetector;

impl EnvironmentDetector {
    /// Detect the host environment.
    pub fn detect() -> EnvironmentProfile {
        let mut profile = EnvironmentProfile::default();

        profile.os = if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Other
        };

        profile.arch = if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else if cfg!(target_arch = "arm") {
            Arch::Arm
        } else {
            Arch::X86_64
        };

        // CI detection via standard env vars (observation only).
        for key in ["CI", "GITHUB_ACTIONS", "GITLAB_CI", "TRAVIS", "JENKINS_URL"] {
            if std::env::var(key).map_or(false, |v| !v.is_empty()) {
                profile.ci = true;
                break;
            }
        }

        // Container detection (best-effort).
        profile.container = std::env::var("CONTAINER").map_or(false, |v| !v.is_empty())
            || std::path::Path::new("/.dockerenv").exists();

        // Toolchain availability (fast, PATH lookup, no spawning).
        for tool in [
            "cargo", "node", "npm", "python", "python3", "go", "docker", "make", "cmake",
        ] {
            if Self::on_path(tool) {
                profile.available_tools.push(tool.to_string());
            }
        }

        profile.shell = std::env::var("SHELL").ok();
        profile.term = std::env::var("TERM").ok();

        profile
    }

    /// Cheap check whether `cmd` exists on PATH. Uses `which` if present,
    /// otherwise falls back to a bare existence probe.
    fn on_path(cmd: &str) -> bool {
        if let Ok(path) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path) {
                if dir.join(cmd).exists() {
                    return true;
                }
            }
        }
        false
    }
}

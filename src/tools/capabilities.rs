//! Tool Capability Model
//!
//! Defines typed capability flags for tools, enabling fine-grained permission
//! enforcement, capability-based routing, and tool classification.

use serde::{Deserialize, Serialize};

/// Bitmask of tool capabilities.
///
/// Each flag represents a class of operation the tool can perform.
/// Capabilities are used by the permission system, router, and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolCapabilities {
    /// Can read files from the filesystem.
    pub reads_files: bool,
    /// Can write/create/modify files on the filesystem.
    pub writes_files: bool,
    /// Can execute shell commands or system processes.
    pub executes_commands: bool,
    /// Can make network requests (HTTP, TCP, etc.).
    pub accesses_network: bool,
    /// Can read or modify environment variables.
    pub accesses_environment: bool,
    /// Can modify program or system state (databases, configs, etc.).
    pub modifies_state: bool,
    /// Requires explicit user confirmation before execution.
    pub requires_confirmation: bool,
    /// Supports streaming output (async chunk delivery).
    pub streams_output: bool,
}

impl ToolCapabilities {
    /// Returns true if the tool is read-only (no writes, no execution, no state changes).
    pub fn is_read_only(&self) -> bool {
        !self.writes_files
            && !self.executes_commands
            && !self.modifies_state
            && !self.accesses_environment
    }

    /// Returns true if the tool is considered high-risk and requires confirmation.
    pub fn is_high_risk(&self) -> bool {
        self.requires_confirmation
            || (self.executes_commands && self.writes_files)
            || (self.accesses_network && self.modifies_state)
    }

    /// Returns true if the tool can modify anything on the system.
    pub fn is_mutating(&self) -> bool {
        self.writes_files || self.executes_commands || self.modifies_state
    }

    /// Returns true if the tool produces output that can be streamed.
    pub fn supports_streaming(&self) -> bool {
        self.streams_output
    }

    /// Check if this tool's capabilities are a subset of another.
    pub fn is_subset_of(&self, other: &ToolCapabilities) -> bool {
        (!self.reads_files || other.reads_files)
            && (!self.writes_files || other.writes_files)
            && (!self.executes_commands || other.executes_commands)
            && (!self.accesses_network || other.accesses_network)
            && (!self.accesses_environment || other.accesses_environment)
            && (!self.modifies_state || other.modifies_state)
            && (!self.requires_confirmation || other.requires_confirmation)
            && (!self.streams_output || other.streams_output)
    }

    /// Compute the union of two capability sets.
    pub fn union(&self, other: &ToolCapabilities) -> ToolCapabilities {
        ToolCapabilities {
            reads_files: self.reads_files || other.reads_files,
            writes_files: self.writes_files || other.writes_files,
            executes_commands: self.executes_commands || other.executes_commands,
            accesses_network: self.accesses_network || other.accesses_network,
            accesses_environment: self.accesses_environment || other.accesses_environment,
            modifies_state: self.modifies_state || other.modifies_state,
            requires_confirmation: self.requires_confirmation || other.requires_confirmation,
            streams_output: self.streams_output || other.streams_output,
        }
    }

    /// Compute the intersection of two capability sets.
    pub fn intersection(&self, other: &ToolCapabilities) -> ToolCapabilities {
        ToolCapabilities {
            reads_files: self.reads_files && other.reads_files,
            writes_files: self.writes_files && other.writes_files,
            executes_commands: self.executes_commands && other.executes_commands,
            accesses_network: self.accesses_network && other.accesses_network,
            accesses_environment: self.accesses_environment && other.accesses_environment,
            modifies_state: self.modifies_state && other.modifies_state,
            requires_confirmation: self.requires_confirmation && other.requires_confirmation,
            streams_output: self.streams_output && other.streams_output,
        }
    }
}

/// Categories of tools based on their capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToolCategory {
    #[default]
    Unknown,
    /// Read-only information gathering (list files, read files, git status).
    Informational,
    /// File mutations (create, edit, patch).
    Mutating,
    /// Command execution (shell, build, test).
    Executable,
    /// Network operations (HTTP requests, API calls).
    Network,
    /// State management (database, config changes).
    Stateful,
    /// Composite tools that combine multiple categories.
    Composite,
}

impl ToolCategory {
    /// Derive the category from tool capabilities.
    pub fn from_capabilities(caps: &ToolCapabilities) -> Self {
        let mut count = 0u32;
        if caps.reads_files {
            count += 1;
        }
        if caps.writes_files {
            count += 1;
        }
        if caps.executes_commands {
            count += 1;
        }
        if caps.accesses_network {
            count += 1;
        }
        if caps.modifies_state {
            count += 1;
        }

        match (
            caps.reads_files,
            caps.writes_files,
            caps.executes_commands,
            caps.accesses_network,
            caps.modifies_state,
        ) {
            (true, false, false, false, false) => ToolCategory::Informational,
            (true, true, false, false, false) => ToolCategory::Mutating,
            (false, false, true, false, false) => ToolCategory::Executable,
            (false, false, false, true, false) => ToolCategory::Network,
            (_, _, _, _, true) => ToolCategory::Stateful,
            _ if count > 1 => ToolCategory::Composite,
            _ => ToolCategory::Unknown,
        }
    }
}

/// Permission policy for a tool based on its capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionPolicy {
    /// Tool is always allowed (read-only, no side effects).
    AutoAllow,
    /// Tool requires explicit user confirmation.
    RequireConfirmation,
    /// Tool is blocked by policy.
    Blocked,
    /// Permission is managed by an external system.
    External,
}

impl ToolCapabilities {
    /// Determine the default permission policy for this capability set.
    pub fn permission_policy(&self) -> PermissionPolicy {
        if self.is_read_only() && !self.requires_confirmation {
            PermissionPolicy::AutoAllow
        } else if self.is_high_risk() {
            PermissionPolicy::RequireConfirmation
        } else if self.requires_confirmation {
            PermissionPolicy::RequireConfirmation
        } else {
            PermissionPolicy::AutoAllow
        }
    }

    /// Format capabilities as a human-readable string.
    pub fn format(&self) -> String {
        let mut parts = Vec::new();
        if self.reads_files {
            parts.push("read");
        }
        if self.writes_files {
            parts.push("write");
        }
        if self.executes_commands {
            parts.push("execute");
        }
        if self.accesses_network {
            parts.push("network");
        }
        if self.accesses_environment {
            parts.push("env");
        }
        if self.modifies_state {
            parts.push("state");
        }
        if self.streams_output {
            parts.push("stream");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_only_capabilities() {
        let caps = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        assert!(caps.is_read_only());
        assert_eq!(caps.permission_policy(), PermissionPolicy::AutoAllow);
        assert_eq!(
            ToolCategory::from_capabilities(&caps),
            ToolCategory::Informational
        );
    }

    #[test]
    fn test_mutating_capabilities() {
        let caps = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            ..Default::default()
        };
        assert!(!caps.is_read_only());
        assert!(caps.is_mutating());
        assert_eq!(caps.permission_policy(), PermissionPolicy::AutoAllow);
        assert_eq!(
            ToolCategory::from_capabilities(&caps),
            ToolCategory::Mutating
        );
    }

    #[test]
    fn test_high_risk_capabilities() {
        let caps = ToolCapabilities {
            executes_commands: true,
            writes_files: true,
            ..Default::default()
        };
        assert!(caps.is_high_risk());
        assert_eq!(
            caps.permission_policy(),
            PermissionPolicy::RequireConfirmation
        );
    }

    #[test]
    fn test_subset() {
        let a = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        let b = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            ..Default::default()
        };
        assert!(a.is_subset_of(&b));
        assert!(!b.is_subset_of(&a));
    }

    #[test]
    fn test_union() {
        let a = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        let b = ToolCapabilities {
            writes_files: true,
            ..Default::default()
        };
        let union = a.union(&b);
        assert!(union.reads_files);
        assert!(union.writes_files);
    }

    #[test]
    fn test_intersection() {
        let a = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            ..Default::default()
        };
        let b = ToolCapabilities {
            reads_files: true,
            executes_commands: true,
            ..Default::default()
        };
        let inter = a.intersection(&b);
        assert!(inter.reads_files);
        assert!(!inter.writes_files);
        assert!(!inter.executes_commands);
    }

    #[test]
    fn test_format() {
        let caps = ToolCapabilities {
            reads_files: true,
            writes_files: true,
            executes_commands: true,
            ..Default::default()
        };
        let formatted = caps.format();
        assert!(formatted.contains("read"));
        assert!(formatted.contains("write"));
        assert!(formatted.contains("execute"));
    }

    #[test]
    fn test_empty_format() {
        let caps = ToolCapabilities::default();
        assert_eq!(caps.format(), "none");
    }
}

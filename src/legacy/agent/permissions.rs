#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionLevel {
    Allow,
    Deny,
    Ask,
}

impl Default for PermissionLevel {
    fn default() -> Self {
        PermissionLevel::Ask
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool_name: String,
    pub level: PermissionLevel,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionConfig {
    pub rules: Vec<PermissionRule>,
    pub default_level: PermissionLevel,
    pub dangerous_patterns: Vec<String>,
}

pub struct PermissionManager {
    config: PermissionConfig,
    config_path: PathBuf,
}

impl PermissionManager {
    pub fn new(config_dir: PathBuf) -> Result<Self> {
        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .with_context(|| format!("Failed to create config directory: {:?}", config_dir))?;
        }

        let config_path = config_dir.join("permissions.json");

        let config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .with_context(|| "Failed to read permissions config")?;
            serde_json::from_str(&content).with_context(|| "Failed to parse permissions config")?
        } else {
            PermissionConfig {
                rules: Vec::new(),
                default_level: PermissionLevel::Ask,
                dangerous_patterns: vec![
                    "delete_file".to_string(),
                    "rm ".to_string(),
                    "rm -rf".to_string(),
                    "git push".to_string(),
                    "git push".to_string(),
                    "git reset --hard".to_string(),
                    "git clean".to_string(),
                    "shutdown".to_string(),
                    "reboot".to_string(),
                    "format".to_string(),
                    "chmod -R".to_string(),
                    "rm -".to_string(),
                ],
            }
        };

        Ok(PermissionManager {
            config,
            config_path,
        })
    }

    pub fn check_permission(&self, tool_name: &str, args: &str) -> PermissionDecision {
        if let Some(rule) = self.config.rules.iter().find(|r| r.tool_name == tool_name) {
            return PermissionDecision {
                tool_name: tool_name.to_string(),
                decision: rule.level.clone(),
                reason: rule.reason.clone(),
            };
        }

        for pattern in &self.config.dangerous_patterns {
            if tool_name.contains(pattern) || args.contains(pattern) {
                return PermissionDecision {
                    tool_name: tool_name.to_string(),
                    decision: PermissionLevel::Ask,
                    reason: Some(format!("Action matches dangerous pattern: {}", pattern)),
                };
            }
        }

        let safe_tools = ["list_files", "read_file", "git_status", "git_diff"];
        if safe_tools.contains(&tool_name) {
            return PermissionDecision {
                tool_name: tool_name.to_string(),
                decision: PermissionLevel::Allow,
                reason: None,
            };
        }

        PermissionDecision {
            tool_name: tool_name.to_string(),
            decision: self.config.default_level.clone(),
            reason: None,
        }
    }

    pub fn allow(&mut self, tool_name: &str) -> Result<()> {
        if let Some(rule) = self
            .config
            .rules
            .iter_mut()
            .find(|r| r.tool_name == tool_name)
        {
            rule.level = PermissionLevel::Allow;
        } else {
            self.config.rules.push(PermissionRule {
                tool_name: tool_name.to_string(),
                level: PermissionLevel::Allow,
                reason: Some("Explicitly allowed by user".to_string()),
            });
        }
        self.save()?;
        Ok(())
    }

    pub fn deny(&mut self, tool_name: &str) -> Result<()> {
        if let Some(rule) = self
            .config
            .rules
            .iter_mut()
            .find(|r| r.tool_name == tool_name)
        {
            rule.level = PermissionLevel::Deny;
        } else {
            self.config.rules.push(PermissionRule {
                tool_name: tool_name.to_string(),
                level: PermissionLevel::Deny,
                reason: Some("Explicitly denied by user".to_string()),
            });
        }
        self.save()?;
        Ok(())
    }

    pub fn set_default_level(&mut self, level: PermissionLevel) -> Result<()> {
        self.config.default_level = level;
        self.save()?;
        Ok(())
    }

    pub fn list_rules(&self) -> Vec<&PermissionRule> {
        self.config.rules.iter().collect()
    }

    pub fn is_dangerous(&self, tool_name: &str, args: &str) -> bool {
        for pattern in &self.config.dangerous_patterns {
            if tool_name.contains(pattern) || args.contains(pattern) {
                return true;
            }
        }
        false
    }

    fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.config)
            .with_context(|| "Failed to serialize permissions config")?;
        fs::write(&self.config_path, content)
            .with_context(|| "Failed to write permissions config")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PermissionDecision {
    pub tool_name: String,
    pub decision: PermissionLevel,
    pub reason: Option<String>,
}

impl PermissionDecision {
    pub fn is_allowed(&self) -> bool {
        self.decision == PermissionLevel::Allow
    }

    pub fn requires_ask(&self) -> bool {
        self.decision == PermissionLevel::Ask
    }

    pub fn is_denied(&self) -> bool {
        self.decision == PermissionLevel::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_tool_auto_allowed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = PermissionManager::new(dir.path().join("perms")).unwrap();

        let decision = manager.check_permission("list_files", ".");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_read_file_auto_allowed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = PermissionManager::new(dir.path().join("perms")).unwrap();

        let decision = manager.check_permission("read_file", "src/main.rs");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_dangerous_tool_requires_ask() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = PermissionManager::new(dir.path().join("perms")).unwrap();

        let decision = manager.check_permission("delete_file", "src/main.rs");
        assert!(decision.requires_ask());
    }

    #[test]
    fn test_allow_permission() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut manager = PermissionManager::new(dir.path().join("perms")).unwrap();

        manager.allow("custom_tool").unwrap();
        let decision = manager.check_permission("custom_tool", "");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_deny_permission() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut manager = PermissionManager::new(dir.path().join("perms")).unwrap();

        manager.deny("dangerous_tool").unwrap();
        let decision = manager.check_permission("dangerous_tool", "");
        assert!(decision.is_denied());
    }

    #[test]
    fn test_dangerous_pattern_detection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = PermissionManager::new(dir.path().join("perms")).unwrap();

        assert!(manager.is_dangerous("run_command", "rm -rf /"));
        assert!(manager.is_dangerous("run_command", "git push origin main"));
        assert!(!manager.is_dangerous("run_command", "echo hello"));
    }

    #[test]
    fn test_permission_decision_properties() {
        let decision = PermissionDecision {
            tool_name: "test".to_string(),
            decision: PermissionLevel::Allow,
            reason: None,
        };
        assert!(decision.is_allowed());
        assert!(!decision.requires_ask());
        assert!(!decision.is_denied());
    }
}

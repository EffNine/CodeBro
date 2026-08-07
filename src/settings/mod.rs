//! Settings Manager
//!
//! Provides an interactive, TUI-integrated settings management system.
//! All configuration is managed through the TUI — no manual file editing required.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::Config;

// ─── Settings Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettingKind {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Select {
        options: Vec<String>,
        default: usize,
    },
}

impl std::fmt::Display for SettingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingKind::String(s) => write!(f, "{}", s),
            SettingKind::Integer(i) => write!(f, "{}", i),
            SettingKind::Float(fl) => write!(f, "{}", fl),
            SettingKind::Boolean(b) => write!(f, "{}", b),
            SettingKind::Select {
                options,
                default: _,
            } => {
                write!(f, "[{}]", options.join(", "))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub display_name: String,
    pub description: String,
    pub kind: SettingKind,
    pub value: SettingKind,
    pub section: SettingSection,
    pub modified: bool,
    pub requires_restart: bool,
}

impl Setting {
    pub fn new(
        key: &str,
        display_name: &str,
        description: &str,
        kind: SettingKind,
        value: SettingKind,
        section: SettingSection,
    ) -> Self {
        Setting {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            kind,
            value,
            section,
            modified: false,
            requires_restart: false,
        }
    }

    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    pub fn reset(&mut self) {
        self.value = self.kind.clone();
        self.modified = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingSection {
    General,
    Provider,
    Workspace,
    Features,
    Advanced,
}

impl std::fmt::Display for SettingSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingSection::General => write!(f, "General"),
            SettingSection::Provider => write!(f, "Provider"),
            SettingSection::Workspace => write!(f, "Workspace"),
            SettingSection::Features => write!(f, "Features"),
            SettingSection::Advanced => write!(f, "Advanced"),
        }
    }
}

// ─── Settings Manager ────────────────────────────────────────────────────────

pub struct SettingsManager {
    settings: Vec<Setting>,
    config: Config,
    config_dir: PathBuf,
    pending_changes: Vec<usize>,
}

impl SettingsManager {
    pub fn new(config: Config, config_dir: PathBuf) -> Self {
        let mut manager = SettingsManager {
            settings: Vec::new(),
            config,
            config_dir,
            pending_changes: Vec::new(),
        };
        manager.build_default_settings();
        manager
    }

    fn build_default_settings(&mut self) {
        // Provider settings
        self.settings.push(Setting::new(
            "provider",
            "Provider",
            "AI provider to use (openai, openrouter, deepseek, ollama, lmstudio)",
            SettingKind::Select {
                options: vec![
                    "openai".to_string(),
                    "openrouter".to_string(),
                    "deepseek".to_string(),
                    "ollama".to_string(),
                    "lmstudio".to_string(),
                ],
                default: 0,
            },
            SettingKind::String(self.config.provider.clone()),
            SettingSection::Provider,
        ));

        self.settings.push(Setting::new(
            "base_url",
            "Base URL",
            "API endpoint base URL",
            SettingKind::String(String::new()),
            SettingKind::String(self.config.base_url.clone()),
            SettingSection::Provider,
        ));

        self.settings.push(Setting::new(
            "model",
            "Model",
            "Model to use for completions",
            SettingKind::String(String::new()),
            SettingKind::String(if self.config.model.is_empty() {
                "auto-detect".to_string()
            } else {
                self.config.model.clone()
            }),
            SettingSection::Provider,
        ));

        // General settings
        self.settings.push(Setting::new(
            "context_token_budget",
            "Context Token Budget",
            "Maximum tokens to use for context building",
            SettingKind::Integer(8000),
            SettingKind::Integer(8000),
            SettingSection::General,
        ));

        self.settings.push(Setting::new(
            "max_tool_iterations",
            "Max Tool Iterations",
            "Maximum number of tool call rounds per task",
            SettingKind::Integer(5),
            SettingKind::Integer(5),
            SettingSection::General,
        ));

        self.settings.push(Setting::new(
            "auto_approve_safe",
            "Auto-Approve Safe Operations",
            "Automatically approve safe operations (read, list)",
            SettingKind::Boolean(false),
            SettingKind::Boolean(false),
            SettingSection::General,
        ));

        // Feature flags
        self.settings.push(Setting::new(
            "show_coordination",
            "Show Coordination Panel",
            "Display the agent coordination view",
            SettingKind::Boolean(false),
            SettingKind::Boolean(false),
            SettingSection::Features,
        ));

        self.settings.push(Setting::new(
            "show_task_graph",
            "Show Task Graph",
            "Display the task execution graph",
            SettingKind::Boolean(false),
            SettingKind::Boolean(false),
            SettingSection::Features,
        ));

        self.settings.push(Setting::new(
            "show_metrics",
            "Show Metrics Panel",
            "Display task metrics (tokens, cost, time)",
            SettingKind::Boolean(true),
            SettingKind::Boolean(true),
            SettingSection::Features,
        ));

        self.settings.push(Setting::new(
            "show_memory_notifications",
            "Show Memory Notifications",
            "Display memory update notifications",
            SettingKind::Boolean(true),
            SettingKind::Boolean(true),
            SettingSection::Features,
        ));

        self.settings.push(Setting::new(
            "show_skill_notifications",
            "Show Skill Notifications",
            "Display skill confidence changes",
            SettingKind::Boolean(true),
            SettingKind::Boolean(true),
            SettingSection::Features,
        ));
    }

    // ─── Getters ───────────────────────────────────────────────────────────

    pub fn get_setting(&self, key: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.key == key)
    }

    pub fn get_settings_by_section(&self, section: &SettingSection) -> Vec<&Setting> {
        self.settings
            .iter()
            .filter(|s| &s.section == section)
            .collect()
    }

    pub fn get_all_settings(&self) -> &[Setting] {
        &self.settings
    }

    pub fn modified_settings(&self) -> Vec<usize> {
        self.pending_changes.clone()
    }

    pub fn has_pending_changes(&self) -> bool {
        !self.pending_changes.is_empty()
    }

    // ─── Setters ───────────────────────────────────────────────────────────

    pub fn set_string(&mut self, key: &str, value: &str) -> Result<()> {
        if let Some(idx) = self.settings.iter().position(|s| s.key == key) {
            if let SettingKind::String(_) = self.settings[idx].kind {
                self.settings[idx].value = SettingKind::String(value.to_string());
                self.settings[idx].mark_modified();
                if !self.pending_changes.contains(&idx) {
                    self.pending_changes.push(idx);
                }
                return Ok(());
            }
        }
        anyhow::bail!("Setting '{}' not found or wrong type", key);
    }

    pub fn set_integer(&mut self, key: &str, value: i64) -> Result<()> {
        if let Some(idx) = self.settings.iter().position(|s| s.key == key) {
            if let SettingKind::Integer(_) = self.settings[idx].kind {
                self.settings[idx].value = SettingKind::Integer(value);
                self.settings[idx].mark_modified();
                if !self.pending_changes.contains(&idx) {
                    self.pending_changes.push(idx);
                }
                return Ok(());
            }
        }
        anyhow::bail!("Setting '{}' not found or wrong type", key);
    }

    pub fn set_boolean(&mut self, key: &str, value: bool) -> Result<()> {
        if let Some(idx) = self.settings.iter().position(|s| s.key == key) {
            if let SettingKind::Boolean(_) = self.settings[idx].kind {
                self.settings[idx].value = SettingKind::Boolean(value);
                self.settings[idx].mark_modified();
                if !self.pending_changes.contains(&idx) {
                    self.pending_changes.push(idx);
                }
                return Ok(());
            }
        }
        anyhow::bail!("Setting '{}' not found or wrong type", key);
    }

    pub fn set_select(&mut self, key: &str, index: usize) -> Result<()> {
        if let Some(idx) = self.settings.iter().position(|s| s.key == key) {
            if let SettingKind::Select { options, .. } = &self.settings[idx].kind {
                if index < options.len() {
                    self.settings[idx].value = SettingKind::String(options[index].clone());
                    self.settings[idx].mark_modified();
                    if !self.pending_changes.contains(&idx) {
                        self.pending_changes.push(idx);
                    }
                    return Ok(());
                }
            }
        }
        anyhow::bail!("Setting '{}' not found or invalid index", key);
    }

    // ─── Apply & Discard ───────────────────────────────────────────────────

    pub fn apply_changes(&mut self) -> Result<()> {
        for &idx in &self.pending_changes {
            let key = self.settings[idx].key.clone();
            let value = self.settings[idx].value.clone();

            match (&key[..], &value) {
                ("provider", SettingKind::String(v)) => {
                    self.config.provider = v.clone();
                }
                ("base_url", SettingKind::String(v)) => {
                    self.config.base_url = v.clone();
                }
                ("model", SettingKind::String(v)) => {
                    self.config.model = v.clone();
                }
                ("context_token_budget", SettingKind::Integer(v)) => {
                    // Store in a side channel for now
                }
                ("max_tool_iterations", SettingKind::Integer(v)) => {
                    // Store in a side channel for now
                }
                ("auto_approve_safe", SettingKind::Boolean(v)) => {
                    // Store in a side channel for now
                }
                _ => {}
            }

            self.settings[idx].modified = false;
        }

        // Persist to config file
        self.config.persist_model()?;

        self.pending_changes.clear();
        Ok(())
    }

    pub fn discard_changes(&mut self) {
        for &idx in &self.pending_changes {
            self.settings[idx].reset();
        }
        self.pending_changes.clear();
    }

    // ─── Summary ───────────────────────────────────────────────────────────

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push("CodeBro Settings".to_string());
        lines.push("─────────────────".to_string());

        let sections = [
            SettingSection::General,
            SettingSection::Provider,
            SettingSection::Workspace,
            SettingSection::Features,
            SettingSection::Advanced,
        ];

        for section in &sections {
            let section_settings = self.get_settings_by_section(section);
            if section_settings.is_empty() {
                continue;
            }

            lines.push(format!("\n[{}] ", section));
            for setting in section_settings {
                let modified = if setting.modified { " *" } else { "" };
                lines.push(format!(
                    "  {}{}: {}",
                    setting.display_name, modified, setting.value
                ));
            }
        }

        if self.has_pending_changes() {
            lines.push("\n* Modified (not yet applied)".to_string());
        }

        lines.join("\n")
    }
}

// ─── Settings Panel State (for TUI) ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsView {
    List,
    Edit(String),
    Confirm,
}

#[derive(Debug, Clone)]
pub struct SettingsPanel {
    pub view: SettingsView,
    pub selected_section: SettingSection,
    pub selected_setting_index: usize,
    pub edit_buffer: String,
    pub scroll_offset: usize,
}

impl SettingsPanel {
    pub fn new() -> Self {
        SettingsPanel {
            view: SettingsView::List,
            selected_section: SettingSection::General,
            selected_setting_index: 0,
            edit_buffer: String::new(),
            scroll_offset: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        !matches!(self.view, SettingsView::List) || self.selected_section != SettingSection::General
    }

    pub fn open(&mut self) {
        self.view = SettingsView::List;
        self.selected_section = SettingSection::General;
        self.selected_setting_index = 0;
        self.edit_buffer.clear();
        self.scroll_offset = 0;
    }

    pub fn close(&mut self) {
        self.view = SettingsView::List;
        self.edit_buffer.clear();
    }

    pub fn toggle_section(&mut self, section: &SettingSection) {
        self.selected_section = section.clone();
        self.selected_setting_index = 0;
        self.view = SettingsView::List;
        self.edit_buffer.clear();
    }

    pub fn start_edit(&mut self, key: &str) {
        self.view = SettingsView::Edit(key.to_string());
        if let Some(setting) = self.find_setting(key) {
            self.edit_buffer = match &setting.value {
                SettingKind::String(s) => s.clone(),
                SettingKind::Integer(i) => i.to_string(),
                SettingKind::Float(fl) => fl.to_string(),
                SettingKind::Boolean(b) => b.to_string(),
                SettingKind::Select { options, default } => {
                    options.get(*default).cloned().unwrap_or_default()
                }
            };
        }
    }

    pub fn finish_edit(&mut self) {
        self.view = SettingsView::List;
        self.edit_buffer.clear();
    }

    pub fn cancel_edit(&mut self) {
        self.view = SettingsView::List;
        self.edit_buffer.clear();
    }

    fn find_setting(&self, key: &str) -> Option<Setting> {
        // This needs access to SettingsManager; we'll use a callback instead
        None
    }
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_manager_creation() {
        let config = Config {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
        };
        let manager = SettingsManager::new(config, PathBuf::from("/tmp"));
        assert!(!manager.get_all_settings().is_empty());
    }

    #[test]
    fn test_setting_sections() {
        let config = Config::default_test();
        let manager = SettingsManager::new(config, PathBuf::from("/tmp"));

        let general = manager.get_settings_by_section(&SettingSection::General);
        let provider = manager.get_settings_by_section(&SettingSection::Provider);
        let features = manager.get_settings_by_section(&SettingSection::Features);

        assert!(!general.is_empty());
        assert!(!provider.is_empty());
        assert!(!features.is_empty());
    }

    #[test]
    fn test_set_and_get() {
        let config = Config::default_test();
        let mut manager = SettingsManager::new(config, PathBuf::from("/tmp"));

        assert!(manager.set_string("model", "gpt-4o-mini").is_ok());
        let setting = manager.get_setting("model").unwrap();
        assert_eq!(
            setting.value,
            SettingKind::String("gpt-4o-mini".to_string())
        );
        assert!(setting.modified);
    }

    #[test]
    fn test_pending_changes() {
        let config = Config::default_test();
        let mut manager = SettingsManager::new(config, PathBuf::from("/tmp"));

        assert!(!manager.has_pending_changes());
        manager.set_string("model", "test").unwrap();
        assert!(manager.has_pending_changes());
        manager.discard_changes();
        assert!(!manager.has_pending_changes());
    }

    #[test]
    fn test_summary() {
        let config = Config::default_test();
        let manager = SettingsManager::new(config, PathBuf::from("/tmp"));
        let summary = manager.summary();
        assert!(summary.contains("CodeBro Settings"));
        assert!(summary.contains("Provider"));
    }
}

// Helper for tests
impl Config {
    pub fn default_test() -> Self {
        Config {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
        }
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub root: String,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub active_files: Vec<String>,
    pub recent_commands: Vec<String>,
    pub metadata: HashMap<String, String>,
}

impl WorkspaceInfo {
    pub fn new(root: &str) -> Self {
        WorkspaceInfo {
            root: root.to_string(),
            language: None,
            framework: None,
            active_files: Vec::new(),
            recent_commands: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn add_active_file(&mut self, file: String) {
        self.active_files.retain(|f| f != &file);
        self.active_files.insert(0, file);
    }

    pub fn add_recent_file(&mut self, file: String) {
        self.active_files.retain(|f| f != &file);
        self.active_files.insert(0, file);
    }

    pub fn add_recent_command(&mut self, command: String) {
        self.recent_commands.retain(|c| c != &command);
        self.recent_commands.insert(0, command);
    }

    pub fn set_language(&mut self, language: String) {
        self.language = Some(language);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceManager {
    pub info: WorkspaceInfo,
    pub workspace_dir: PathBuf,
}

impl WorkspaceManager {
    pub fn new(workspace_path: PathBuf) -> Result<Self> {
        let info = if workspace_path.exists() {
            let content = fs::read_to_string(&workspace_path)?;
            serde_json::from_str(&content)?
        } else {
            WorkspaceInfo {
                root: String::new(),
                language: None,
                framework: None,
                active_files: Vec::new(),
                recent_commands: Vec::new(),
                metadata: HashMap::new(),
            }
        };

        Ok(WorkspaceManager {
            info,
            workspace_dir: workspace_path,
        })
    }

    pub fn info(&self) -> &WorkspaceInfo {
        &self.info
    }

    pub fn track_file(&mut self, file: &str) -> Result<()> {
        if !self.info.active_files.contains(&file.to_string()) {
            self.info.active_files.push(file.to_string());
        }
        self.save()?;
        Ok(())
    }

    pub fn track_file_access(&mut self, file: &str) -> Result<()> {
        self.track_file(file)
    }

    pub fn detect_project(&mut self, root: &str) -> Result<()> {
        self.info.root = root.to_string();
        self.save()
    }

    pub fn track_command(&mut self, command: &str) -> Result<()> {
        self.info.recent_commands.insert(0, command.to_string());
        self.info.recent_commands.truncate(20);
        self.save()?;
        Ok(())
    }

    pub fn set_language(&mut self, language: &str) {
        self.info.language = Some(language.to_string());
    }

    pub fn set_framework(&mut self, framework: &str) {
        self.info.framework = Some(framework.to_string());
    }

    pub fn set_root(&mut self, root: &str) {
        self.info.root = root.to_string();
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.workspace_dir.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(&self.info)?;
        fs::write(&self.workspace_dir, content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceArtifact {
    pub key: String,
    pub agent: String,
    pub content: String,
    pub version: u32,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub artifacts: HashMap<String, WorkspaceArtifact>,
    pub history: Vec<HashMap<String, WorkspaceArtifact>>,
    pub workspace_dir: PathBuf,
}

impl Workspace {
    pub fn new() -> Result<Self, anyhow::Error> {
        let workspace_dir = crate::config::Config::config_dir().join("workspace");
        std::fs::create_dir_all(&workspace_dir)?;
        Ok(Workspace {
            artifacts: HashMap::new(),
            history: Vec::new(),
            workspace_dir,
        })
    }

    pub fn write_artifact(
        &mut self,
        key: &str,
        agent: &str,
        content: &str,
    ) -> Result<(), anyhow::Error> {
        let version = self.artifacts.get(key).map(|a| a.version + 1).unwrap_or(1);

        let artifact = WorkspaceArtifact {
            key: key.to_string(),
            agent: agent.to_string(),
            content: content.to_string(),
            version,
            timestamp: chrono::Local::now().to_rfc3339(),
        };

        self.artifacts.insert(key.to_string(), artifact);
        self.save()?;
        Ok(())
    }

    pub fn read_artifact(&self, key: &str) -> Option<String> {
        self.artifacts.get(key).map(|a| a.content.clone())
    }

    pub fn update_artifact(&mut self, key: &str, content: &str) -> Result<(), anyhow::Error> {
        if let Some(artifact) = self.artifacts.get_mut(key) {
            artifact.content = content.to_string();
            artifact.version += 1;
            artifact.timestamp = chrono::Local::now().to_rfc3339();
            self.save()?;
        }
        Ok(())
    }

    pub fn get_history(&self) -> Vec<&HashMap<String, WorkspaceArtifact>> {
        self.history.iter().collect()
    }

    pub fn snapshot(&mut self) {
        let current = self.artifacts.clone();
        self.history.push(current);
        while self.history.len() > 10 {
            self.history.remove(0);
        }
    }

    fn save(&self) -> Result<(), anyhow::Error> {
        let path = self.workspace_dir.join("artifacts.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn list_keys(&self) -> Vec<String> {
        self.artifacts.keys().cloned().collect()
    }
}

pub fn get_workspace_path() -> PathBuf {
    crate::config::Config::config_dir().join("workspace")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_artifact_write() {
        let mut ws = Workspace::new().unwrap();
        ws.write_artifact("plan", "planning", "Implement caching")
            .unwrap();
        assert_eq!(ws.read_artifact("plan").unwrap(), "Implement caching");
    }

    #[test]
    fn test_workspace_version_increment() {
        let mut ws = Workspace::new().unwrap();
        ws.write_artifact("key1", "agent1", "v1").unwrap();
        ws.write_artifact("key1", "agent1", "v2").unwrap();
        assert_eq!(ws.read_artifact("key1").unwrap(), "v2");
    }

    #[test]
    fn test_workspace_snapshot() {
        let mut ws = Workspace::new().unwrap();
        ws.write_artifact("a", "x", "data").unwrap();
        ws.snapshot();
        assert_eq!(ws.get_history().len(), 1);
    }
}

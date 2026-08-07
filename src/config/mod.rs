#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_dir = Self::config_dir();
        let config_path = config_dir.join("config.toml");

        let mut config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {:?}", config_path))?
        } else {
            Config {
                provider: "openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                // Empty model means "auto-detect from the provider" on startup.
                model: String::new(),
                api_key: None,
            }
        };

        if let Ok(val) = env::var("CODEBRO_API_KEY") {
            config.api_key = Some(val);
        }
        if let Ok(val) = env::var("CODEBRO_BASE_URL") {
            config.base_url = val;
        }
        if let Ok(val) = env::var("CODEBRO_MODEL") {
            config.model = val;
        }

        Ok(config)
    }

    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codebro")
    }

    /// Whether the user has explicitly chosen a model (via file or env var).
    pub fn is_model_set(&self) -> bool {
        !self.model.trim().is_empty()
    }

    /// Persists `provider`, `base_url`, and `model` back to `~/.codebro/config.toml`,
    /// preserving any existing `api_key` already stored in the file. Used so an
    /// auto-detected model is remembered across launches.
    pub fn persist_model(&self) -> Result<()> {
        let config_dir = Self::config_dir();
        let config_path = config_dir.join("config.toml");

        let mut value: toml::Value = if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
            toml::from_str(&content).unwrap_or_else(|_| toml::Value::Table(Default::default()))
        } else {
            toml::Value::Table(Default::default())
        };

        let table = value
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config file is not a TOML table"))?;
        table.insert(
            "provider".to_string(),
            toml::Value::String(self.provider.clone()),
        );
        table.insert(
            "base_url".to_string(),
            toml::Value::String(self.base_url.clone()),
        );
        table.insert("model".to_string(), toml::Value::String(self.model.clone()));

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .with_context(|| format!("Failed to create config dir: {:?}", config_dir))?;
        }
        let content = toml::to_string(&value)?;
        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write config file: {:?}", config_path))?;
        Ok(())
    }

    pub fn ensure_config_dir() -> Result<()> {
        let dir = Self::config_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create config directory: {:?}", dir))?;
        }
        Ok(())
    }

    /// Load config from a specific directory (for testing).
    pub fn load_from_dir(dir: &std::path::Path) -> Result<Self> {
        let config_path = dir.join("config.toml");
        let mut config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {:?}", config_path))?
        } else {
            Config {
                provider: "openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                model: String::new(),
                api_key: None,
            }
        };

        if let Ok(val) = env::var("CODEBRO_API_KEY") {
            config.api_key = Some(val);
        }
        if let Ok(val) = env::var("CODEBRO_BASE_URL") {
            config.base_url = val;
        }
        if let Ok(val) = env::var("CODEBRO_MODEL") {
            config.model = val;
        }

        Ok(config)
    }

    /// Persist config to a specific directory (for testing).
    pub fn persist_to_dir(&self, dir: &std::path::Path) -> Result<()> {
        let config_path = dir.join("config.toml");
        let content = format!(
            "provider = \"{}\"\nbase_url = \"{}\"\nmodel = \"{}\"\n",
            self.provider, self.base_url, self.model
        );
        fs::create_dir_all(dir)?;
        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write config file: {:?}", config_path))?;
        Ok(())
    }
}

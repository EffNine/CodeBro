//! Provider Manager
//!
//! Manages API keys, provider switching, model selection, health status,
//! and connection testing. This is the backbone of the Developer Experience
//! Platform's provider management capabilities.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::providers::{discover_model, fetch_models, Provider};

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderId {
    OpenAI,
    OpenRouter,
    DeepSeek,
    Ollama,
    LMStudio,
    Custom(String),
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderId::OpenAI => write!(f, "OpenAI"),
            ProviderId::OpenRouter => write!(f, "OpenRouter"),
            ProviderId::DeepSeek => write!(f, "DeepSeek"),
            ProviderId::Ollama => write!(f, "Ollama"),
            ProviderId::LMStudio => write!(f, "LM Studio"),
            ProviderId::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl ProviderId {
    pub fn as_str(&self) -> &str {
        match self {
            ProviderId::OpenAI => "openai",
            ProviderId::OpenRouter => "openrouter",
            ProviderId::DeepSeek => "deepseek",
            ProviderId::Ollama => "ollama",
            ProviderId::LMStudio => "lmstudio",
            ProviderId::Custom(s) => s,
        }
    }

    pub fn default_base_url(&self) -> String {
        match self {
            ProviderId::OpenAI => "https://api.openai.com/v1".to_string(),
            ProviderId::OpenRouter => "https://openrouter.ai/api/v1".to_string(),
            ProviderId::DeepSeek => "https://api.deepseek.com/v1".to_string(),
            ProviderId::Ollama => "http://localhost:11434".to_string(),
            ProviderId::LMStudio => "http://localhost:1234/v1".to_string(),
            ProviderId::Custom(_) => String::new(),
        }
    }

    pub fn from_str(s: &str) -> Option<ProviderId> {
        match s.to_lowercase().as_str() {
            "openai" => Some(ProviderId::OpenAI),
            "openrouter" => Some(ProviderId::OpenRouter),
            "deepseek" => Some(ProviderId::DeepSeek),
            "ollama" => Some(ProviderId::Ollama),
            "lmstudio" | "lm studio" => Some(ProviderId::LMStudio),
            "" => None,
            _ => Some(ProviderId::Custom(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Unhealthy { reason: String },
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Unknown => write!(f, "Unknown"),
            HealthStatus::Healthy => write!(f, "Healthy"),
            HealthStatus::Unhealthy { reason } => write!(f, "Unhealthy: {}", reason),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub id: ProviderId,
    pub base_url: String,
    pub api_key: Option<String>,
    pub current_model: String,
    pub health: HealthStatus,
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
    pub latency_ms: Option<u64>,
}

impl ProviderEntry {
    pub fn new(id: ProviderId, base_url: String) -> Self {
        ProviderEntry {
            id: id.clone(),
            base_url,
            api_key: None,
            current_model: String::new(),
            health: HealthStatus::Unknown,
            last_health_check: None,
            latency_ms: None,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some() && !self.base_url.is_empty()
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.health, HealthStatus::Healthy)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub is_default: bool,
}

// ─── ProviderManager ─────────────────────────────────────────────────────────

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ProviderManager {
    providers: HashMap<String, ProviderEntry>,
    active_provider: Option<String>,
    config_dir: PathBuf,
}

impl ProviderManager {
    pub fn new(config_dir: PathBuf) -> Self {
        ProviderManager {
            config_dir,
            ..Default::default()
        }
    }

    // ─── Registration ────────────────────────────────────────────────────

    pub fn register_builtin(&mut self) {
        for id in [
            ProviderId::OpenAI,
            ProviderId::OpenRouter,
            ProviderId::DeepSeek,
            ProviderId::Ollama,
            ProviderId::LMStudio,
        ] {
            let entry = ProviderEntry::new(id.clone(), id.default_base_url());
            self.providers.insert(id.as_str().to_string(), entry);
        }
    }

    pub fn register_custom(&mut self, id: ProviderId, base_url: String) {
        let key = id.as_str().to_string();
        if !self.providers.contains_key(&key) {
            self.providers.insert(key, ProviderEntry::new(id, base_url));
        }
    }

    // ─── Active Provider ─────────────────────────────────────────────────

    pub fn set_active(&mut self, provider_id: &str) -> Result<()> {
        if !self.providers.contains_key(provider_id) {
            anyhow::bail!("Provider '{}' not found", provider_id);
        }
        self.active_provider = Some(provider_id.to_string());
        Ok(())
    }

    pub fn active_id(&self) -> Option<&ProviderId> {
        self.active_provider
            .as_ref()
            .and_then(|k| self.providers.get(k).map(|p| &p.id))
    }

    pub fn active_provider(&self) -> Option<&String> {
        self.active_provider.as_ref()
    }

    pub fn active_base_url(&self) -> Option<&str> {
        self.active_provider
            .as_ref()
            .and_then(|k| self.providers.get(k).map(|p| p.base_url.as_str()))
    }

    pub fn active_model(&self) -> String {
        self.active_provider
            .as_ref()
            .and_then(|k| self.providers.get(k))
            .map(|p| p.current_model.clone())
            .unwrap_or_default()
    }

    pub fn set_model(&mut self, model: &str) -> Result<()> {
        let key = self
            .active_provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No active provider"))?
            .clone();
        self.providers
            .get_mut(&key)
            .ok_or_else(|| anyhow::anyhow!("Provider not found"))?
            .current_model = model.to_string();
        Ok(())
    }

    // ─── API Keys ────────────────────────────────────────────────────────

    pub fn set_api_key(&mut self, provider_id: &str, key: &str) -> Result<()> {
        if key.is_empty() {
            anyhow::bail!("API key cannot be empty");
        }
        let entry = self
            .providers
            .get_mut(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;
        entry.api_key = Some(key.to_string());
        Ok(())
    }

    pub fn clear_api_key(&mut self, provider_id: &str) -> Result<()> {
        let entry = self
            .providers
            .get_mut(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;
        entry.api_key = None;
        Ok(())
    }

    pub fn has_api_key(&self, provider_id: &str) -> bool {
        self.providers
            .get(provider_id)
            .map(|p| p.api_key.is_some())
            .unwrap_or(false)
    }

    pub fn api_key_masked(&self, provider_id: &str) -> Option<String> {
        self.providers
            .get(provider_id)
            .and_then(|p| p.api_key.as_ref())
            .map(|k| {
                let chars: Vec<char> = k.chars().collect();
                if chars.len() <= 4 {
                    String::from("••••")
                } else {
                    format!(
                        "••••{}",
                        chars[chars.len() - 4..].iter().collect::<String>()
                    )
                }
            })
    }

    // ─── Health Checks ───────────────────────────────────────────────────

    pub async fn check_health(&mut self, provider_id: &str) -> Result<HealthStatus> {
        let entry = self
            .providers
            .get_mut(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;

        let start = Instant::now();
        let key = entry.api_key.as_deref();
        let url = &entry.base_url;

        match fetch_models(url, key).await {
            Ok(models) => {
                let latency = start.elapsed().as_millis() as u64;
                entry.latency_ms = Some(latency);
                entry.last_health_check = Some(chrono::Utc::now());
                entry.health = if models.is_empty() {
                    HealthStatus::Unhealthy {
                        reason: "Provider returned no models".to_string(),
                    }
                } else {
                    HealthStatus::Healthy
                };
                Ok(entry.health.clone())
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                entry.latency_ms = Some(latency);
                entry.last_health_check = Some(chrono::Utc::now());
                entry.health = HealthStatus::Unhealthy {
                    reason: e.to_string(),
                };
                Ok(entry.health.clone())
            }
        }
    }

    pub async fn check_all_health(&mut self) -> Vec<(String, HealthStatus, Option<u64>)> {
        let mut results = Vec::new();
        let keys: Vec<String> = self.providers.keys().cloned().collect();
        for key in keys {
            let health = match self.check_health(&key).await {
                Ok(h) => h.clone(),
                Err(_) => self
                    .providers
                    .get(&key)
                    .map(|p| p.health.clone())
                    .unwrap_or(HealthStatus::Unknown),
            };
            let latency = self.providers.get(&key).and_then(|p| p.latency_ms);
            results.push((key, health, latency));
        }
        results
    }

    pub fn get_health(&self, provider_id: &str) -> &HealthStatus {
        self.providers
            .get(provider_id)
            .map(|p| &p.health)
            .unwrap_or(&HealthStatus::Unknown)
    }

    // ─── Models ──────────────────────────────────────────────────────────

    pub async fn fetch_models(&mut self, provider_id: &str) -> Result<Vec<ModelInfo>> {
        let entry = self
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;

        let models = fetch_models(&entry.base_url, entry.api_key.as_deref()).await?;
        let default_model = crate::providers::pick_default(&models);

        Ok(models
            .into_iter()
            .map(|m| ModelInfo {
                id: m.clone(),
                is_default: default_model.as_deref() == Some(&m),
            })
            .collect())
    }

    pub fn discover_default_model(&mut self, provider_id: &str) -> Option<String> {
        let entry = self.providers.get(provider_id)?.clone();
        let key = entry.api_key.as_deref().map(|s| s.to_string());
        let url = entry.base_url.clone();

        // Run discovery in a separate thread to avoid runtime nesting issues
        let key_clone = key.clone();
        let url_clone = url.clone();
        let discovered = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return None,
            };
            rt.block_on(crate::providers::discover_model(
                &url_clone,
                key_clone.as_deref(),
            ))
        })
        .join()
        .ok()
        .flatten();

        discovered
    }

    // ─── List Providers ──────────────────────────────────────────────────

    pub fn list_providers(&self) -> Vec<(&str, &ProviderEntry)> {
        self.providers
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }

    pub fn list_provider_ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    // ─── Build Provider Instance ─────────────────────────────────────────

    pub fn build_provider(&self) -> Result<Box<dyn Provider>> {
        let key = self
            .active_provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No active provider"))?;
        let entry = self
            .providers
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("Active provider not found"))?;

        Ok(Box::new(crate::providers::OpenAiProvider::new(
            crate::config::Config {
                provider: entry.id.as_str().to_string(),
                base_url: entry.base_url.clone(),
                model: entry.current_model.clone(),
                api_key: entry.api_key.clone(),
            },
        )))
    }

    // ─── Persistence ─────────────────────────────────────────────────────

    pub fn persist(&self) -> Result<()> {
        let config_path = self.config_dir.join("providers.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, json)
            .with_context(|| format!("Failed to write providers to {:?}", config_path))?;
        Ok(())
    }

    pub fn load(&mut self) -> Result<()> {
        let config_path = self.config_dir.join("providers.json");
        if !config_path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read providers from {:?}", config_path))?;
        let loaded: ProviderManager =
            serde_json::from_str(&content).with_context(|| "Failed to parse providers.json")?;
        self.providers = loaded.providers;
        self.active_provider = loaded.active_provider;
        Ok(())
    }
}

// ─── Wizard State ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum WizardStep {
    SelectProvider,
    EnterApiKey,
    SelectModel,
    Confirm,
    Complete,
}

#[derive(Debug, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub selected_provider: Option<ProviderId>,
    pub base_url: String,
    pub api_key_input: String,
    pub api_key_confirmed: bool,
    pub models: Vec<ModelInfo>,
    pub model_filter: String,
    pub selected_model: Option<String>,
    pub error: Option<String>,
}

impl WizardState {
    pub fn new() -> Self {
        WizardState {
            step: WizardStep::SelectProvider,
            selected_provider: None,
            base_url: String::new(),
            api_key_input: String::new(),
            api_key_confirmed: false,
            models: Vec::new(),
            model_filter: String::new(),
            selected_model: None,
            error: None,
        }
    }

    pub fn select_provider(&mut self, provider: &ProviderId) {
        self.selected_provider = Some(provider.clone());
        self.base_url = provider.default_base_url();
        self.step = WizardStep::EnterApiKey;
    }

    pub fn set_api_key(&mut self, key: &str) {
        self.api_key_input = key.to_string();
    }

    pub fn confirm_api_key(&mut self) {
        self.api_key_confirmed = true;
        self.step = WizardStep::SelectModel;
    }

    pub fn set_model_filter(&mut self, filter: &str) {
        self.model_filter = filter.to_string();
    }

    pub fn select_model(&mut self, model: &str) {
        self.selected_model = Some(model.to_string());
    }

    pub fn confirm_selection(&mut self) -> bool {
        self.selected_provider.is_some() && self.api_key_confirmed && self.selected_model.is_some()
    }

    pub fn visible_models(&self) -> Vec<&ModelInfo> {
        let f = self.model_filter.to_lowercase();
        if f.is_empty() {
            self.models.iter().collect()
        } else {
            self.models
                .iter()
                .filter(|m| m.id.to_lowercase().contains(&f))
                .collect()
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.step, WizardStep::Complete)
    }
}

impl Default for WizardState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_id_display() {
        assert_eq!(ProviderId::OpenAI.to_string(), "OpenAI");
        assert_eq!(
            ProviderId::Custom("myprovider".to_string()).to_string(),
            "myprovider"
        );
    }

    #[test]
    fn test_provider_id_from_str() {
        assert_eq!(ProviderId::from_str("openai"), Some(ProviderId::OpenAI));
        assert_eq!(
            ProviderId::from_str("custom"),
            Some(ProviderId::Custom("custom".to_string()))
        );
        assert_eq!(ProviderId::from_str(""), None);
    }

    #[test]
    fn test_provider_id_default_urls() {
        assert_eq!(
            ProviderId::OpenAI.default_base_url(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            ProviderId::Ollama.default_base_url(),
            "http://localhost:11434"
        );
    }

    #[test]
    fn test_wizard_flow() {
        let mut wizard = WizardState::new();
        assert_eq!(wizard.step, WizardStep::SelectProvider);

        wizard.select_provider(&ProviderId::OpenAI);
        assert_eq!(wizard.step, WizardStep::EnterApiKey);
        assert_eq!(wizard.base_url, "https://api.openai.com/v1");

        wizard.set_api_key("sk-test123");
        wizard.confirm_api_key();
        assert_eq!(wizard.step, WizardStep::SelectModel);
        assert!(wizard.api_key_confirmed);

        wizard.select_model("gpt-4o");
        assert_eq!(wizard.selected_model, Some("gpt-4o".to_string()));
    }

    #[test]
    fn test_wizard_confirm_selection() {
        let mut wizard = WizardState::new();
        assert!(!wizard.confirm_selection());

        wizard.select_provider(&ProviderId::OpenAI);
        wizard.set_api_key("sk-test");
        wizard.confirm_api_key();
        wizard.select_model("gpt-4o");
        assert!(wizard.confirm_selection());
    }

    #[test]
    fn test_wizard_model_filter() {
        let mut wizard = WizardState::new();
        wizard.models = vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                is_default: true,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                is_default: false,
            },
            ModelInfo {
                id: "whisper-1".to_string(),
                is_default: false,
            },
        ];

        assert_eq!(wizard.visible_models().len(), 3);
        wizard.set_model_filter("gpt");
        let filtered: Vec<&str> = wizard
            .visible_models()
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(filtered, vec!["gpt-4o", "gpt-4o-mini"]);
        assert_eq!(wizard.visible_models().len(), 2);
    }

    #[test]
    fn test_api_key_masking() {
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.set_api_key("openai", "sk-1234567890abcdef").unwrap();
        assert_eq!(pm.api_key_masked("openai"), Some("••••cdef".to_string()));
    }
}

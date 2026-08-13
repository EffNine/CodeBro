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
    AGNES,
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
            ProviderId::AGNES => write!(f, "AGNES"),
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
            ProviderId::AGNES => "agnes",
            ProviderId::Ollama => "ollama",
            ProviderId::LMStudio => "lmstudio",
            ProviderId::Custom(s) => s,
        }
    }

    pub fn default_base_url(&self) -> String {
        match self {
            ProviderId::OpenAI => "https://api.openai.com/v1".to_string(),
            ProviderId::OpenRouter => "https://openrouter.ai/api/v1".to_string(),
            // Official DeepSeek base URL (the bare host; `/v1` is a legacy
            // suffix the API accepts but the official docs no longer use).
            ProviderId::DeepSeek => "https://api.deepseek.com".to_string(),
            ProviderId::AGNES => "https://apihub.agnes-ai.com/v1".to_string(),
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
            "agnes" => Some(ProviderId::AGNES),
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
    /// In-memory API key. Never serialized to `providers.json` — keys live in
    /// the dedicated secure credential store (`credentials.json`, mode 0600).
    #[serde(skip)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub is_default: bool,
    /// Human-friendly display name; `None` when not known.
    pub display_name: Option<String>,
    /// Officially documented tool-calling support; `None` when unknown.
    pub tool_calling: Option<bool>,
    /// Officially documented context window in tokens; `None` when unknown.
    pub context_tokens: Option<u64>,
    /// How this model became known to CodeBro.
    pub source: crate::providers::ModelSource,
}

impl ModelInfo {
    pub fn from_discovered(m: &crate::providers::DiscoveredModel, is_default: bool) -> Self {
        ModelInfo {
            id: m.id.clone(),
            is_default,
            display_name: m.metadata.display_name.clone(),
            tool_calling: m.metadata.tool_calling,
            context_tokens: m.metadata.context_tokens,
            source: m.source,
        }
    }
}

// ─── ProviderManager ─────────────────────────────────────────────────────────

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ProviderManager {
    providers: HashMap<String, ProviderEntry>,
    active_provider: Option<String>,
    config_dir: PathBuf,
    /// Secure credential store; never serialized into `providers.json`.
    #[serde(skip)]
    credentials: crate::credentials::CredentialStore,
}

impl ProviderManager {
    pub fn new(config_dir: PathBuf) -> Self {
        let mut credentials = crate::credentials::CredentialStore::new(config_dir.clone());
        let _ = credentials.load();
        ProviderManager {
            config_dir,
            credentials,
            ..Default::default()
        }
    }

    // ─── Registration ────────────────────────────────────────────────────

    pub fn register_builtin(&mut self) {
        for id in [
            ProviderId::OpenAI,
            ProviderId::OpenRouter,
            ProviderId::DeepSeek,
            ProviderId::AGNES,
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

    /// Read-only access to a registered provider entry.
    pub fn provider_entry(&self, provider_id: &str) -> Option<&ProviderEntry> {
        self.providers.get(provider_id)
    }

    /// The stored API key for a provider, if any (in-memory secret; callers
    /// must never log or render it).
    pub fn api_key_for(&self, provider_id: &str) -> Option<&str> {
        self.providers
            .get(provider_id)
            .and_then(|p| p.api_key.as_deref())
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
        // Secrets go to the dedicated secure store, never to providers.json.
        self.credentials.set(provider_id, key)?;
        Ok(())
    }

    pub fn clear_api_key(&mut self, provider_id: &str) -> Result<()> {
        let entry = self
            .providers
            .get_mut(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;
        entry.api_key = None;
        self.credentials.delete(provider_id)?;
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

    /// Check a provider's health by querying its `/models` endpoint.
    ///
    /// Failures are classified and surfaced with actionable reasons:
    ///
    /// - auth failures (401/403) and quota failures (402) are always
    ///   `Unhealthy` — a stored key is invalid or has no balance;
    /// - rate limiting (429) is `Unhealthy`;
    /// - a broken/missing endpoint is `Unhealthy`, EXCEPT when the provider
    ///   has a deterministic official catalog (e.g. DeepSeek): the provider
    ///   is then marked `Healthy` because CodeBro can still serve its known
    ///   models (labelled `provider default`, never `discovered`).
    pub async fn check_health(&mut self, provider_id: &str) -> Result<HealthStatus> {
        let entry = self
            .providers
            .get_mut(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;

        let start = Instant::now();
        let key = entry.api_key.as_deref().map(|s| s.to_string());
        let url = entry.base_url.clone();
        let provider = provider_id.to_string();

        let outcome = crate::providers::fetch_models_raw(&url, key.as_deref()).await;
        let latency = start.elapsed().as_millis() as u64;
        entry.latency_ms = Some(latency);
        entry.last_health_check = Some(chrono::Utc::now());

        let health = match &outcome {
            Ok(models) if !models.is_empty() => HealthStatus::Healthy,
            Ok(_) => HealthStatus::Unhealthy {
                reason: "Provider returned no models".to_string(),
            },
            Err(err) => {
                let human = err.human();
                match err {
                    crate::providers::DiscoveryError::Http(401)
                    | crate::providers::DiscoveryError::Http(402)
                    | crate::providers::DiscoveryError::Http(403)
                    | crate::providers::DiscoveryError::Http(429) => {
                        HealthStatus::Unhealthy { reason: human }
                    }
                    // Endpoint down (network/5xx/404) but the provider has a
                    // known official catalog: still usable via the fallback.
                    _ if crate::providers::fallback_catalog(&provider).is_some() => {
                        HealthStatus::Healthy
                    }
                    _ => HealthStatus::Unhealthy { reason: human },
                }
            }
        };
        entry.health = health.clone();
        Ok(health)
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

    /// Discover the provider's models: `/models` first, then the
    /// provider-known fallback catalog when the endpoint is unavailable or
    /// incomplete. The provenance (discovered vs provider default) is
    /// attached to every model.
    pub async fn fetch_models(&mut self, provider_id: &str) -> Result<Vec<ModelInfo>> {
        let entry = self
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;

        let base_url = entry.base_url.clone();
        let api_key = entry.api_key.clone();
        let provider = provider_id.to_string();

        let discovery =
            crate::providers::discover_models(&base_url, api_key.as_deref(), &provider).await;
        let ids: Vec<String> = discovery.models.iter().map(|m| m.id.clone()).collect();
        let default_model = crate::providers::pick_default(&ids);

        Ok(discovery
            .models
            .iter()
            .map(|m| ModelInfo::from_discovered(m, default_model.as_deref() == Some(&m.id)))
            .collect())
    }

    pub fn discover_default_model(&mut self, provider_id: &str) -> Option<String> {
        let entry = self.providers.get(provider_id)?.clone();
        let key = entry.api_key.as_deref().map(|s| s.to_string());
        let url = entry.base_url.clone();
        let provider = provider_id.to_string();

        // Run discovery in a separate thread to avoid runtime nesting issues
        let key_clone = key.clone();
        let url_clone = url.clone();
        let provider_clone = provider.clone();
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
                &provider_clone,
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

    /// Providers in a stable, human-friendly order: built-ins first
    /// (OpenAI, OpenRouter, DeepSeek, AGNES, Ollama, LM Studio), then custom
    /// providers alphabetically. Deterministic across calls.
    pub fn list_providers_ordered(&self) -> Vec<(String, &ProviderEntry)> {
        const KNOWN: [&str; 6] = [
            "openai",
            "openrouter",
            "deepseek",
            "agnes",
            "ollama",
            "lmstudio",
        ];
        let mut out = Vec::new();
        for id in KNOWN {
            if let Some(entry) = self.providers.get(id) {
                out.push((id.to_string(), entry));
            }
        }
        let mut rest: Vec<String> = self
            .providers
            .keys()
            .filter(|k| !KNOWN.contains(&k.as_str()))
            .cloned()
            .collect();
        rest.sort();
        for id in rest {
            if let Some(entry) = self.providers.get(&id) {
                out.push((id, entry));
            }
        }
        out
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

    /// Persist provider configuration. API keys are excluded by the
    /// `#[serde(skip)]` on [`ProviderEntry::api_key`]; secrets live in the
    /// secure credential store (`credentials.json`, mode 0600). A legacy
    /// `providers.json` that still contains inline keys is migrated on load
    /// and never re-written here.
    pub fn persist(&self) -> Result<()> {
        let config_path = self.config_dir.join("providers.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, json)
            .with_context(|| format!("Failed to write providers to {:?}", config_path))?;
        // Keep the credential file in sync even if only a key was set.
        let _ = self.credentials.persist();
        Ok(())
    }

    pub fn load(&mut self) -> Result<()> {
        let config_path = self.config_dir.join("providers.json");
        if !config_path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read providers from {:?}", config_path))?;

        // Migration: legacy files may contain inline `api_key` values. Move
        // them into the secure credential store, then treat them as if they
        // were already stored there. Existing credentials are never destroyed.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(providers) = value.get("providers").and_then(|v| v.as_object()) {
                for (id, entry) in providers {
                    if let Some(key) = entry.get("api_key").and_then(|k| k.as_str()) {
                        if !key.trim().is_empty() {
                            let _ = self.credentials.set(id, key);
                        }
                    }
                }
            }
        }

        let loaded: ProviderManager =
            serde_json::from_str(&content).with_context(|| "Failed to parse providers.json")?;
        // Merge, never replace: entries registered locally (e.g. built-ins
        // added by `register_builtin` like AGNES) survive even when the
        // persisted file predates them. Persisted values (base URL, health,
        // model) win for entries that already exist.
        for (key, value) in loaded.providers {
            match self.providers.get_mut(&key) {
                Some(existing) => {
                    existing.base_url = value.base_url;
                    existing.current_model = value.current_model;
                    existing.health = value.health;
                    existing.last_health_check = value.last_health_check;
                    existing.latency_ms = value.latency_ms;
                }
                None => {
                    self.providers.insert(key, value);
                }
            }
        }
        self.active_provider = loaded.active_provider;

        // Merge stored credentials back into memory so providers keep working
        // without being re-entered.
        for id in self.providers.keys().cloned().collect::<Vec<_>>() {
            if let Some(key) = self.credentials.get(&id).map(|k| k.to_string()) {
                if let Some(entry) = self.providers.get_mut(&id) {
                    entry.api_key = Some(key);
                }
            }
        }
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
        assert_eq!(ProviderId::AGNES.to_string(), "AGNES");
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
            ProviderId::DeepSeek.default_base_url(),
            "https://api.deepseek.com"
        );
        assert_eq!(
            ProviderId::AGNES.default_base_url(),
            "https://apihub.agnes-ai.com/v1"
        );
        assert_eq!(
            ProviderId::Ollama.default_base_url(),
            "http://localhost:11434"
        );
    }

    #[test]
    fn test_provider_id_agnes_roundtrip() {
        assert_eq!(ProviderId::from_str("agnes"), Some(ProviderId::AGNES));
        assert_eq!(ProviderId::AGNES.as_str(), "agnes");
        assert_eq!(ProviderId::AGNES.to_string(), "AGNES");
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
                display_name: None,
                tool_calling: None,
                context_tokens: None,
                source: crate::providers::ModelSource::Discovered,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                is_default: false,
                display_name: None,
                tool_calling: None,
                context_tokens: None,
                source: crate::providers::ModelSource::Discovered,
            },
            ModelInfo {
                id: "whisper-1".to_string(),
                is_default: false,
                display_name: None,
                tool_calling: None,
                context_tokens: None,
                source: crate::providers::ModelSource::Discovered,
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

    #[test]
    fn test_api_key_not_persisted_to_providers_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_dir = dir.path().to_path_buf();

        let mut pm = ProviderManager::new(config_dir.clone());
        pm.register_builtin();
        pm.set_api_key("openai", "sk-super-secret-key-1234567890")
            .unwrap();
        pm.set_active("openai").unwrap();
        pm.persist().unwrap();

        let providers_json = std::fs::read_to_string(config_dir.join("providers.json")).unwrap();
        assert!(
            !providers_json.contains("sk-super-secret-key-1234567890"),
            "API key leaked into providers.json: {}",
            providers_json
        );

        // The key lives in the dedicated credential file.
        let credentials_json =
            std::fs::read_to_string(config_dir.join("credentials.json")).unwrap();
        assert!(credentials_json.contains("sk-super-secret-key-1234567890"));
    }

    #[test]
    fn test_legacy_providers_json_migrates_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_dir = dir.path().to_path_buf();
        // Simulate a legacy plaintext providers.json.
        let legacy = serde_json::json!({
            "providers": {
                "openai": {
                    "id": "OpenAI",
                    "base_url": "https://api.openai.com/v1",
                    "api_key": "sk-legacy-plaintext-key",
                    "current_model": "",
                    "health": "Unknown",
                    "last_health_check": null,
                    "latency_ms": null
                }
            },
            "active_provider": "openai",
            "config_dir": "/tmp"
        });
        std::fs::write(config_dir.join("providers.json"), legacy.to_string()).unwrap();

        let mut pm = ProviderManager::new(config_dir.clone());
        pm.register_builtin();
        pm.load().unwrap();

        assert!(
            pm.has_api_key("openai"),
            "migrated key must be usable in memory"
        );
        assert_eq!(pm.api_key_masked("openai"), Some("••••-key".to_string()));
    }

    #[test]
    fn test_clear_api_key_removes_from_credentials() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_dir = dir.path().to_path_buf();
        let mut pm = ProviderManager::new(config_dir.clone());
        pm.register_builtin();
        pm.set_api_key("openai", "sk-1234567890abcdef").unwrap();
        pm.clear_api_key("openai").unwrap();
        assert!(!pm.has_api_key("openai"));
        let mut reloaded = ProviderManager::new(config_dir.clone());
        reloaded.register_builtin();
        reloaded.load().unwrap();
        assert!(!reloaded.has_api_key("openai"));
    }

    #[test]
    fn test_builtin_registration_includes_agnes() {
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        let ids = pm.list_provider_ids();
        assert!(ids.iter().any(|id| id == "agnes"), "AGNES registered");
        assert!(ids.iter().any(|id| id == "deepseek"));
        assert!(ids.iter().any(|id| id == "ollama"));
    }

    #[test]
    fn test_configured_status_distinguishes_local_and_cloud() {
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        // Cloud providers need a key to be "configured".
        assert!(!pm.has_api_key("deepseek"));
        pm.set_api_key("deepseek", "sk-ds-test-123456").unwrap();
        assert!(pm.has_api_key("deepseek"));
        // Local providers exist without keys.
        let ollama = pm.providers.get("ollama").unwrap();
        assert!(!ollama.base_url.is_empty());
        assert!(
            crate::providers::is_local_provider("ollama"),
            "Ollama is a local provider"
        );
    }

    // ─── Health / discovery (deterministic local mock server) ────────────

    async fn mock_server(status: u16, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        url
    }

    #[tokio::test]
    async fn test_health_auth_failure_is_unhealthy_with_actionable_reason() {
        let url = mock_server(401, r#"{"error":"invalid key"}"#).await;
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.set_api_key("deepseek", "sk-wrong").unwrap();
        pm.providers.get_mut("deepseek").unwrap().base_url = url;
        let health = pm.check_health("deepseek").await.unwrap();
        match health {
            HealthStatus::Unhealthy { reason } => {
                assert!(
                    reason.contains("authentication failed"),
                    "auth failure must be actionable, got: {}",
                    reason
                );
            }
            other => panic!("401 must be Unhealthy, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_health_endpoint_down_uses_fallback_for_deepseek() {
        // Nothing is listening on this port: connection refused.
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.set_api_key("deepseek", "sk-test-123456").unwrap();
        pm.providers.get_mut("deepseek").unwrap().base_url = "http://127.0.0.1:1".to_string();
        let health = pm.check_health("deepseek").await.unwrap();
        assert_eq!(
            health,
            HealthStatus::Healthy,
            "known catalog keeps provider usable"
        );
    }

    #[tokio::test]
    async fn test_health_endpoint_down_is_unhealthy_without_fallback() {
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.providers.get_mut("openai").unwrap().base_url = "http://127.0.0.1:1".to_string();
        let health = pm.check_health("openai").await.unwrap();
        assert!(
            matches!(health, HealthStatus::Unhealthy { .. }),
            "openai has no fallback catalog; endpoint down must be Unhealthy"
        );
    }

    #[tokio::test]
    async fn test_fetch_models_marks_fallback_as_provider_default() {
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.set_api_key("deepseek", "sk-test-123456").unwrap();
        pm.providers.get_mut("deepseek").unwrap().base_url = "http://127.0.0.1:1".to_string();
        let models = pm.fetch_models("deepseek").await.unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["deepseek-v4-flash", "deepseek-v4-pro"]);
        assert!(
            models
                .iter()
                .all(|m| m.source == crate::providers::ModelSource::ProviderDefault),
            "fallback models must never claim to be discovered"
        );
        assert_eq!(models[0].tool_calling, Some(true));
        assert_eq!(models[0].context_tokens, Some(1_000_000));
    }

    #[tokio::test]
    async fn test_fetch_models_marks_discovered_models() {
        let url = mock_server(
            200,
            r#"{"data":[{"id":"deepseek-v4-flash"},{"id":"deepseek-v4-pro"}]}"#,
        )
        .await;
        let mut pm = ProviderManager::new(PathBuf::from("/tmp"));
        pm.register_builtin();
        pm.set_api_key("deepseek", "sk-test-123456").unwrap();
        pm.providers.get_mut("deepseek").unwrap().base_url = url;
        let models = pm.fetch_models("deepseek").await.unwrap();
        assert!(models.len() >= 2);
        assert!(
            models
                .iter()
                .all(|m| m.source == crate::providers::ModelSource::Discovered),
            "advertised models must be marked discovered"
        );
        // Metadata is unknown for discovered models (never fabricated).
        assert!(models.iter().all(|m| m.tool_calling.is_none()));
        assert!(models.iter().all(|m| m.context_tokens.is_none()));
    }
}
